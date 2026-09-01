//! Poster fallback for a Linux box with no GStreamer H.264 decoder.
//!
//! macOS and Windows decode video with the OS (AVFoundation / Media Foundation).
//! Linux does not: the decoder is a package the user installs, and a stock Ubuntu
//! desktop ships `gstreamer1.0-plugins-{base,good}` and nothing that decodes h264 —
//! so the `playbin` probe returns no frame there at all.
//!
//! Two tiers of recovery, in order:
//!
//! 1. Decode the keyframe ourselves with openh264, which is linked into the binary
//!    and needs nothing installed. It covers 8-bit 4:2:0 Baseline/Main/High, which
//!    is all but a rounding error of what people attach.
//! 2. When even that cannot decode it — hevc, 10-bit, 4:2:2 — report the size the
//!    container declares anyway, so the message renders at the right aspect ratio
//!    instead of the 0x0 box a failed probe used to produce.

use std::fs::File;
use std::io::BufReader;

use openh264::formats::YUVSource;

use crate::VideoProbe;

/// Match the GStreamer path, which seeks to 1s so a fade-in does not become the
/// poster. We take the last keyframe at or before that instead of seeking.
const POSTER_SECONDS: f64 = 1.0;

/// Enough to walk past `POSTER_SECONDS` on any sane frame rate without reading a
/// whole file when a video has no keyframe after the first.
const MAX_SAMPLES_SCANNED: u32 = 300;

/// Frames to feed after the keyframe before giving up: openh264 can ask for more
/// data before it emits the first picture.
const MAX_EXTRA_FEEDS: u32 = 8;

struct VideoTrack {
    id: u32,
    width: u32,
    height: u32,
    timescale: u32,
    is_h264: bool,
    parameter_sets: Vec<u8>,
}

pub(crate) fn probe_without_decoder(path: &str, max_poster_edge: u32) -> Option<VideoProbe> {
    let file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut reader = mp4::Mp4Reader::read_header(BufReader::new(file), size).ok()?;
    let track = video_track(&reader)?;

    let poster_jpeg = if track.is_h264 {
        decode_poster(&mut reader, &track, max_poster_edge)
    } else {
        None
    };
    if poster_jpeg.is_none() {
        tracing::warn!(
            target: "mezon_video",
            width = track.width,
            height = track.height,
            "no poster without a system decoder; sending the container's size only"
        );
    }
    Some(VideoProbe {
        width: track.width,
        height: track.height,
        poster_jpeg,
    })
}

/// Lowest track id wins, so a file with two video tracks probes the same way twice
/// (`tracks()` is a HashMap and iterates in no particular order).
fn video_track<R: std::io::Read + std::io::Seek>(reader: &mp4::Mp4Reader<R>) -> Option<VideoTrack> {
    let mut tracks: Vec<_> = reader.tracks().iter().collect();
    tracks.sort_by_key(|(id, _)| **id);
    tracks.into_iter().find_map(|(id, track)| {
        if track.width() == 0 || track.height() == 0 {
            return None;
        }
        let is_h264 = matches!(track.media_type(), Ok(mp4::MediaType::H264));
        Some(VideoTrack {
            id: *id,
            width: u32::from(track.width()),
            height: u32::from(track.height()),
            timescale: track.timescale(),
            is_h264,
            parameter_sets: is_h264
                .then(|| annex_b_parameter_sets(track))
                .flatten()
                .unwrap_or_default(),
        })
    })
}

/// SPS/PPS live in the `avcC` box, not in the samples, so a keyframe decodes only
/// when they are prepended.
fn annex_b_parameter_sets(track: &mp4::Mp4Track) -> Option<Vec<u8>> {
    let avcc = &track.trak.mdia.minf.stbl.stsd.avc1.as_ref()?.avcc;
    let mut out = Vec::new();
    for nal in avcc
        .sequence_parameter_sets
        .iter()
        .chain(avcc.picture_parameter_sets.iter())
    {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&nal.bytes);
    }
    (!out.is_empty()).then_some(out)
}

fn decode_poster<R: std::io::Read + std::io::Seek>(
    reader: &mut mp4::Mp4Reader<R>,
    track: &VideoTrack,
    max_poster_edge: u32,
) -> Option<Vec<u8>> {
    if track.parameter_sets.is_empty() {
        return None;
    }
    let keyframe = poster_sample(reader, track)?;
    let mut decoder = openh264::decoder::Decoder::new()
        .inspect_err(|error| {
            tracing::warn!(target: "mezon_video", %error, "openh264 decoder init failed");
        })
        .ok()?;

    let mut unit = track.parameter_sets.clone();
    append_annex_b(&mut unit, &keyframe.bytes);
    let first_extra = keyframe.index.saturating_add(1);
    for next in first_extra..=first_extra.saturating_add(MAX_EXTRA_FEEDS) {
        match decoder.decode(&unit) {
            Ok(Some(yuv)) => return encode(&yuv, max_poster_edge),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(target: "mezon_video", %error, "openh264 could not decode this video");
                return None;
            }
        }
        // The picture is not out yet; feed the frame after it and try again.
        let sample = reader.read_sample(track.id, next).ok().flatten()?;
        unit.clear();
        append_annex_b(&mut unit, &sample.bytes);
    }
    None
}

fn encode(yuv: &openh264::decoder::DecodedYUV<'_>, max_poster_edge: u32) -> Option<Vec<u8>> {
    let (width, height) = yuv.dimensions();
    let mut rgb = vec![0u8; width.checked_mul(height)?.checked_mul(3)?];
    yuv.write_rgb8(&mut rgb);
    let rgb =
        image::RgbImage::from_raw(u32::try_from(width).ok()?, u32::try_from(height).ok()?, rgb)?;
    crate::poster::encode_rgb_jpeg(rgb, max_poster_edge)
}

struct PosterSample {
    index: u32,
    bytes: Vec<u8>,
}

/// The last keyframe at or before [`POSTER_SECONDS`], falling back to the first one.
fn poster_sample<R: std::io::Read + std::io::Seek>(
    reader: &mut mp4::Mp4Reader<R>,
    track: &VideoTrack,
) -> Option<PosterSample> {
    let count = reader.sample_count(track.id).ok()?.min(MAX_SAMPLES_SCANNED);
    let mut best: Option<PosterSample> = None;
    for index in 1..=count {
        let Some(sample) = reader.read_sample(track.id, index).ok().flatten() else {
            continue;
        };
        let past_poster_time = sample_seconds(sample.start_time, track.timescale) > POSTER_SECONDS;
        if sample.is_sync {
            best = Some(PosterSample {
                index,
                bytes: sample.bytes.to_vec(),
            });
        }
        if past_poster_time && best.is_some() {
            break;
        }
    }
    best
}

fn sample_seconds(start_time: u64, timescale: u32) -> f64 {
    if timescale == 0 {
        return 0.0;
    }
    start_time as f64 / f64::from(timescale)
}

/// mp4 stores NAL units length-prefixed (AVCC); openh264 wants start codes.
fn append_annex_b(out: &mut Vec<u8>, avcc: &[u8]) {
    let mut at = 0usize;
    while at + 4 <= avcc.len() {
        let len = u32::from_be_bytes([avcc[at], avcc[at + 1], avcc[at + 2], avcc[at + 3]]) as usize;
        at += 4;
        let Some(end) = at.checked_add(len).filter(|end| *end <= avcc.len()) else {
            return;
        };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&avcc[at..end]);
        at = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_annex_b_restamps_every_length_prefixed_unit() {
        let avcc = [0, 0, 0, 2, 0x67, 0xAA, 0, 0, 0, 1, 0x68];
        let mut out = Vec::new();
        append_annex_b(&mut out, &avcc);
        assert_eq!(out, vec![0, 0, 0, 1, 0x67, 0xAA, 0, 0, 0, 1, 0x68]);
    }

    #[test]
    fn append_annex_b_stops_on_a_length_that_runs_past_the_buffer() {
        let mut out = Vec::new();
        append_annex_b(&mut out, &[0, 0, 0, 9, 0x67]);
        assert!(out.is_empty());
    }

    #[test]
    fn append_annex_b_ignores_a_trailing_partial_prefix() {
        let mut out = Vec::new();
        append_annex_b(&mut out, &[0, 0, 0, 1, 0x67, 0, 0]);
        assert_eq!(out, vec![0, 0, 0, 1, 0x67]);
    }

    #[test]
    fn sample_seconds_uses_the_track_timescale_and_survives_a_zero() {
        assert!((sample_seconds(1500, 1000) - 1.5).abs() < f64::EPSILON);
        assert_eq!(sample_seconds(1500, 0), 0.0);
    }
}
