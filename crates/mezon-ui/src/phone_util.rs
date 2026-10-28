pub const COUNTRIES: [(&str, &str); 3] = [("Vietnam", "+84"), ("Japan", "+81"), ("US", "+1")];

pub fn country_dial_code(country_index: usize) -> &'static str {
    COUNTRIES
        .get(country_index)
        .map(|(_, dial)| *dial)
        .unwrap_or("+84")
}

pub fn parse_phone_vn(phone: &str) -> String {
    if let Some(rest) = phone.strip_prefix('0') {
        format!("+84{rest}")
    } else if phone.starts_with("+84") {
        phone.to_string()
    } else {
        format!("+84{phone}")
    }
}

pub fn is_valid_phone_vn(phone: &str) -> bool {
    let parsed = parse_phone_vn(phone);
    let Some(national) = parsed.strip_prefix("+84") else {
        return false;
    };
    let mut chars = national.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '3' | '5' | '7' | '8' | '9') {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    rest.len() == 8 && rest.iter().all(|c| c.is_ascii_digit())
}

fn is_valid_phone_international(raw: &str) -> bool {
    let cleaned = raw.trim();
    cleaned.len() >= 7 && cleaned.chars().all(|c| c.is_ascii_digit())
}

pub fn build_login_phone(country_index: usize, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let dial = country_dial_code(country_index);
    if dial == "+84" {
        if !is_valid_phone_vn(trimmed) {
            return None;
        }
        return Some(parse_phone_vn(trimmed));
    }
    if !is_valid_phone_international(trimmed) {
        return None;
    }
    let national = if trimmed.starts_with('0') {
        trimmed.trim_start_matches('0')
    } else {
        trimmed
    };
    Some(format!("{dial}{national}"))
}

pub fn phone_error_key(country_index: usize, raw: &str) -> Option<&'static str> {
    if raw.trim().is_empty() {
        Some("common.login.enterPhoneToLogin")
    } else if build_login_phone(country_index, raw).is_none() {
        Some("common.login.invalidPhoneNumber")
    } else {
        None
    }
}

pub fn mask_phone(phone: &str) -> String {
    let count = phone.chars().count();
    if count < 3 {
        return phone.to_string();
    }
    let tail: String = phone.chars().skip(count - 2).collect();
    format!("xxx-xxx{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vn_phone_validation_matches_modal_rules() {
        for prefix in ['3', '5', '7', '8', '9'] {
            assert!(is_valid_phone_vn(&format!("0{prefix}12345678")));
        }
        assert!(is_valid_phone_vn("+84912345678"));
        assert!(!is_valid_phone_vn("0112345678"));
        assert!(!is_valid_phone_vn("091234567"));
    }

    #[test]
    fn build_login_phone_vn_and_japan() {
        assert_eq!(
            build_login_phone(0, "0912345678").as_deref(),
            Some("+84912345678")
        );
        assert_eq!(
            build_login_phone(1, "9012345678").as_deref(),
            Some("+819012345678")
        );
        assert!(build_login_phone(0, "123").is_none());
    }
}
