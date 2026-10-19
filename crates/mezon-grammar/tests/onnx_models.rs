use mezon_grammar::client::MezonGrammarClient;
use mezon_grammar::onnx::{INTENT_MIN_CONFIDENCE, apply_neural_accent_onnx, predict_intent};
use mezon_grammar::tokenizer::VietnameseTokenizer;

#[test]
fn intent_leave_or_absence_above_threshold() {
    let tokenizer = VietnameseTokenizer::new();
    let prediction = predict_intent(&tokenizer, "Sáng mai anh nghỉ làm nhé");
    let Some(p) = prediction else {
        return;
    };
    assert!(matches!(
        p.intent.as_str(),
        "action_leave_ack" | "leave_or_absence"
    ));
    assert!(p.confidence >= INTENT_MIN_CONFIDENCE);
}

#[test]
fn intent_other_url_blocks_quick_replies() {
    let client = MezonGrammarClient::new();
    let replies = client.suggest_quick_replies_with_model(
        "https://github.com/mezon/grammar/pull/123",
        None,
        4,
    );
    assert!(replies.is_empty());
}

#[test]
fn neural_accent_restores_vietnamese() {
    let out = apply_neural_accent_onnx("Sang mai anh nghi lam nhe");
    assert!(out.contains('ỉ') || out.contains('á') || out.contains('à'));
}
