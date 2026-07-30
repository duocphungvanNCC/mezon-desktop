use std::default::Default;

use x11rb::protocol::{Event, xproto};
use xim::{
    AHashMap, AttributeName, Client, ClientError, ClientHandler, InputStyle, InputStyleList, Reader,
    XimRead,
};

pub enum XimCallbackEvent {
    XimXEvent(x11rb::protocol::Event),
    XimPreeditEvent(xproto::Window, String),
    XimCommitEvent(xproto::Window, String),
}

fn pick_input_style(attributes: &AHashMap<AttributeName, Vec<u8>>) -> InputStyle {
    let styles = attributes
        .get(&AttributeName::QueryInputStyle)
        .and_then(|bytes| {
            let mut reader = Reader::new(bytes);
            InputStyleList::read(&mut reader).ok()
        })
        .map(|list| list.styles)
        .unwrap_or_default();

    // mezon vendor edit: STATUS_CALLBACKS is never implemented by this handler, so
    // prefer every no-status variant over it.
    let preferred = [
        InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING,
        InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NONE,
        InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_CALLBACKS,
        InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NOTHING,
        InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NONE,
        InputStyle::PREEDIT_POSITION | InputStyle::STATUS_AREA,
        InputStyle::PREEDIT_NOTHING | InputStyle::STATUS_NOTHING,
    ];
    for want in preferred {
        if styles.iter().any(|style| *style == want) {
            return want;
        }
    }
    for style in &styles {
        if style.contains(InputStyle::PREEDIT_CALLBACKS) {
            return InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING;
        }
    }
    for style in &styles {
        if style.contains(InputStyle::PREEDIT_POSITION) {
            return InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NOTHING;
        }
    }
    InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING
}

fn usable_locale(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != "C" && value != "POSIX" && !value.starts_with("C.")
}

fn xim_locale_candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if usable_locale(trimmed) && !candidates.iter().any(|c| c == trimmed) {
                candidates.push(trimmed.to_string());
            }
        }
    }
    for fallback in ["en_US.UTF-8", "C"] {
        if !candidates.iter().any(|c| c == fallback) {
            candidates.push(fallback.to_string());
        }
    }
    candidates
}

pub struct XimHandler {
    pub im_id: u16,
    pub ic_id: u16,
    pub connected: bool,
    pub styles_ready: bool,
    pub opened: bool,
    pub ic_pending: bool,
    locale_candidates: Vec<String>,
    locale_attempt: usize,
    pub window: xproto::Window,
    pub input_style: InputStyle,
    pub last_callback_event: Option<XimCallbackEvent>,
}

impl XimHandler {
    pub fn new() -> Self {
        Self {
            im_id: Default::default(),
            ic_id: Default::default(),
            connected: false,
            styles_ready: false,
            opened: false,
            ic_pending: false,
            locale_candidates: xim_locale_candidates(),
            locale_attempt: 0,
            window: Default::default(),
            input_style: InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING,
            last_callback_event: None,
        }
    }

    // mezon vendor edit: an XIM server that rejects the negotiated locale kills the
    // whole client with a protocol error. Instead of losing IME for the session,
    // retry XIM_OPEN with the next fallback locale (en_US.UTF-8, then C).
    pub fn try_reopen_next_locale<C: Client>(&mut self, client: &mut C) -> bool {
        if self.opened {
            return false;
        }
        self.locale_attempt += 1;
        let Some(locale) = self.locale_candidates.get(self.locale_attempt) else {
            return false;
        };
        eprintln!("[xim] open failed; retrying with locale {locale}");
        client.open(locale).is_ok()
    }
}

impl<C: Client<XEvent = xproto::KeyPressEvent>> ClientHandler<C> for XimHandler {
    fn handle_connect(&mut self, client: &mut C) -> Result<(), ClientError> {
        let locale = self
            .locale_candidates
            .get(self.locale_attempt)
            .cloned()
            .unwrap_or_else(|| "en_US.UTF-8".into());
        client.open(&locale)
    }

    fn handle_open(&mut self, client: &mut C, input_method_id: u16) -> Result<(), ClientError> {
        self.im_id = input_method_id;
        self.opened = true;

        client.get_im_values(input_method_id, &[AttributeName::QueryInputStyle])
    }

    fn handle_get_im_values(
        &mut self,
        client: &mut C,
        input_method_id: u16,
        attributes: AHashMap<AttributeName, Vec<u8>>,
    ) -> Result<(), ClientError> {
        self.input_style = pick_input_style(&attributes);
        self.styles_ready = true;
        if self.window != 0 && self.ic_id == 0 && !self.ic_pending {
            self.ic_pending = true;
            let ic_attributes = client
                .build_ic_attributes()
                .push(AttributeName::InputStyle, self.input_style)
                .push(AttributeName::ClientWindow, self.window)
                .push(AttributeName::FocusWindow, self.window)
                .build();
            client.create_ic(input_method_id, ic_attributes)?;
        }
        Ok(())
    }

    fn handle_create_ic(
        &mut self,
        client: &mut C,
        input_method_id: u16,
        input_context_id: u16,
    ) -> Result<(), ClientError> {
        self.connected = true;
        self.ic_pending = false;
        self.ic_id = input_context_id;
        client.set_focus(input_method_id, input_context_id)?;
        Ok(())
    }

    fn handle_commit(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        text: &str,
    ) -> Result<(), ClientError> {
        self.last_callback_event = Some(XimCallbackEvent::XimCommitEvent(
            self.window,
            String::from(text),
        ));
        Ok(())
    }

    fn handle_forward_event(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        _flag: xim::ForwardEventFlag,
        xev: C::XEvent,
    ) -> Result<(), ClientError> {
        match xev.response_type {
            x11rb::protocol::xproto::KEY_PRESS_EVENT => {
                self.last_callback_event = Some(XimCallbackEvent::XimXEvent(Event::KeyPress(xev)));
            }
            x11rb::protocol::xproto::KEY_RELEASE_EVENT => {
                self.last_callback_event =
                    Some(XimCallbackEvent::XimXEvent(Event::KeyRelease(xev)));
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_close(&mut self, client: &mut C, _input_method_id: u16) -> Result<(), ClientError> {
        client.disconnect()
    }

    fn handle_preedit_draw(
        &mut self,
        _client: &mut C,
        _input_method_id: u16,
        _input_context_id: u16,
        _caret: i32,
        _chg_first: i32,
        _chg_len: i32,
        _status: xim::PreeditDrawStatus,
        preedit_string: &str,
        _feedbacks: Vec<xim::Feedback>,
    ) -> Result<(), ClientError> {
        self.last_callback_event = Some(XimCallbackEvent::XimPreeditEvent(
            self.window,
            String::from(preedit_string),
        ));
        Ok(())
    }
}
