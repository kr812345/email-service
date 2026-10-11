//! Input validation. Everything is checked before a request reaches Resend,
//! so a misbehaving agent can neither send malformed mail nor abuse the API.

use crate::server::SendEmailParams;

/// Resend's own recipient cap per message.
pub const MAX_TOTAL_RECIPIENTS: usize = 50;
/// RFC 5322 maximum header line length.
pub const MAX_SUBJECT_LEN: usize = 998;
/// Cap on each body part so a runaway agent can't push unbounded payloads.
pub const MAX_BODY_BYTES: usize = 1_000_000;
pub const MAX_EMAIL_ID_LEN: usize = 128;

/// Pragmatic address check: bare ASCII `local@domain` only —
/// no display names, comments, or quoted local parts.
pub fn is_valid_email(addr: &str) -> bool {
    if addr.is_empty() || addr.len() > 254 || !addr.is_ascii() {
        return false;
    }
    // No whitespace, control chars, or header/display-name syntax.
    if addr.bytes().any(|b| {
        b <= 0x20 || b == 0x7F || matches!(b, b'<' | b'>' | b'"' | b',' | b'\\' | b'(' | b')')
    }) {
        return false;
    }
    let Some((local, domain)) = addr.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
    {
        return false;
    }
    is_valid_domain(domain)
}

fn is_valid_domain(domain: &str) -> bool {
    if domain.len() > 253 || domain.contains("..") {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    for (i, label) in labels.iter().enumerate() {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return false;
        }
        // Final label (TLD): alphabetic, at least 2 chars.
        if i == labels.len() - 1
            && (label.len() < 2 || !label.bytes().all(|b| b.is_ascii_alphabetic()))
        {
            return false;
        }
    }
    true
}

/// Resend email ids feed directly into the request path (`GET /emails/{id}`),
/// so restrict to a charset that can't traverse or manipulate the URL.
pub fn is_valid_email_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_EMAIL_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn validate_address_list(field: &str, addrs: &[String]) -> Result<(), String> {
    for a in addrs {
        if !is_valid_email(a) {
            return Err(format!(
                "invalid address in `{field}`: {a:?} — bare addresses only, e.g. user@example.com"
            ));
        }
    }
    Ok(())
}

/// Validate the full send_email parameter set. Returns a caller-readable
/// message describing the first problem found.
pub fn validate_send(p: &SendEmailParams) -> Result<(), String> {
    if p.to.is_empty() {
        return Err("`to` must contain at least one recipient".into());
    }

    let total =
        p.to.len() + p.cc.as_deref().map_or(0, <[_]>::len) + p.bcc.as_deref().map_or(0, <[_]>::len);
    if total > MAX_TOTAL_RECIPIENTS {
        return Err(format!(
            "too many recipients: {total} total across to/cc/bcc (max {MAX_TOTAL_RECIPIENTS})"
        ));
    }

    validate_address_list("to", &p.to)?;
    if let Some(cc) = &p.cc {
        validate_address_list("cc", cc)?;
    }
    if let Some(bcc) = &p.bcc {
        validate_address_list("bcc", bcc)?;
    }
    if let Some(reply_to) = &p.reply_to {
        validate_address_list("reply_to", reply_to)?;
    }

    let subject = p.subject.trim();
    if subject.is_empty() {
        return Err("`subject` must not be empty".into());
    }
    if subject.len() > MAX_SUBJECT_LEN {
        return Err(format!(
            "`subject` too long: {} chars (max {MAX_SUBJECT_LEN})",
            subject.len()
        ));
    }
    if p.subject.contains(['\r', '\n']) {
        return Err("`subject` must be a single line".into());
    }

    if p.text.is_none() && p.html.is_none() {
        return Err("at least one of `text` or `html` is required".into());
    }
    for (field, body) in [("text", &p.text), ("html", &p.html)] {
        if let Some(b) = body
            && b.len() > MAX_BODY_BYTES
        {
            return Err(format!(
                "`{field}` body too large: {} bytes (max {MAX_BODY_BYTES})",
                b.len()
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_addresses() {
        for a in [
            "user@example.com",
            "alice@example.com",
            "a.b-c_d+e@sub.domain.co.uk",
            "x@y.io",
        ] {
            assert!(is_valid_email(a), "should accept {a}");
        }
    }

    #[test]
    fn rejects_bad_addresses() {
        for a in [
            "",
            "plainaddress",
            "@no-local.com",
            "no-domain@",
            "two@@at.com",
            "spaces in@address.com",
            "newline@inject.ion\r\nBcc: evil@x.com",
            "display <name@example.com>",
            "quoted\"local@example.com",
            "trailing.dot.@example.com",
            ".leading@example.com",
            "double..dot@example.com",
            "bad..domain@exa..mple.com",
            "no-tld@localhost",
            "numeric-tld@example.123",
            "-hyphen@example.com.e-x",
            "unicode@exámple.com",
            "comma,two@example.com",
        ] {
            assert!(!is_valid_email(a), "should reject {a:?}");
        }
    }

    #[test]
    fn email_id_charset() {
        assert!(is_valid_email_id("4b8e2f10-1234-5678-9abc-def012345678"));
        assert!(is_valid_email_id("re_abc-DEF_123"));
        assert!(!is_valid_email_id(""));
        assert!(!is_valid_email_id("../domains"));
        assert!(!is_valid_email_id("id/with slash"));
        assert!(!is_valid_email_id("id?query=1"));
        assert!(!is_valid_email_id(&"a".repeat(129)));
    }

    fn base_params() -> SendEmailParams {
        SendEmailParams {
            from: "me".into(),
            to: vec!["someone@example.com".into()],
            subject: "hello".into(),
            text: Some("body".into()),
            html: None,
            cc: None,
            bcc: None,
            reply_to: None,
        }
    }

    #[test]
    fn send_validation() {
        assert!(validate_send(&base_params()).is_ok());

        let mut p = base_params();
        p.to.clear();
        assert!(validate_send(&p).unwrap_err().contains("at least one"));

        let mut p = base_params();
        p.to = (0..51).map(|i| format!("u{i}@example.com")).collect();
        assert!(validate_send(&p).unwrap_err().contains("too many"));

        let mut p = base_params();
        p.to = (0..50).map(|i| format!("u{i}@example.com")).collect();
        p.cc = Some(vec!["one-more@example.com".into()]);
        assert!(validate_send(&p).unwrap_err().contains("too many"));

        let mut p = base_params();
        p.cc = Some(vec!["bad address".into()]);
        assert!(validate_send(&p).unwrap_err().contains("`cc`"));

        let mut p = base_params();
        p.subject = "  ".into();
        assert!(validate_send(&p).unwrap_err().contains("subject"));

        let mut p = base_params();
        p.subject = "line1\nBcc: evil@x.com".into();
        assert!(validate_send(&p).unwrap_err().contains("single line"));

        let mut p = base_params();
        p.text = None;
        assert!(validate_send(&p).unwrap_err().contains("text` or `html"));

        let mut p = base_params();
        p.text = Some("x".repeat(MAX_BODY_BYTES + 1));
        assert!(validate_send(&p).unwrap_err().contains("too large"));
    }
}
