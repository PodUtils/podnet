//! The validated share link.

use super::*;

// ---------------------------------------------------------------------------
// Share
// ---------------------------------------------------------------------------

/// Where a share link came from.
///
/// Carried so the operator sees *which* mechanism produced a URL when several
/// are running, and so a link that fails to load can be attributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareSource {
    /// An anonymous quick tunnel from [`crate::quicktunnel`].
    QuickTunnel,
    /// A public URL published by an SSH relay, [`crate::relay`].
    Relay,
    /// A URL the operator supplied.
    Manual,
}

impl ShareSource {
    /// A short label for logs and for the share page.
    pub fn label(&self) -> &'static str {
        match self {
            ShareSource::QuickTunnel => "quick tunnel",
            ShareSource::Relay => "ssh relay",
            ShareSource::Manual => "manual",
        }
    }
}

/// A public URL, validated and rendered for sharing.
///
/// **Why this is a type rather than a `String`.** The URL is the one thing an
/// operator pastes into a chat, a ticket or a terminal, and a malformed one is
/// discovered by a visitor rather than by the process that made it. Building it
/// through a constructor means the checks run once, at the point where the URL
/// enters the program, and every later use is on a value already known good.
///
/// **What is refused.** Credentials in the URL are rejected: `https://user:pass@host`
/// is a real URL that browsers silently accept, and putting one in a shared link
/// publishes the password to everyone the link reaches. Non-HTTP schemes are
/// rejected because a tunnel has no meaning for them, and because a
/// `javascript:` or `file:` URL that reached a QR code becomes a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareLink {
    url: String,
    source: ShareSource,
    note: Option<String>,
}

/// Longest URL accepted. Version 40 QR at error-correction level L holds 2953
/// bytes; this is well inside that, and a longer string is a paste accident.
pub(super) const MAX_SHARE_URL_LEN: usize = 2048;

impl ShareLink {
    /// Validate and wrap a URL.
    pub fn new(url: &str, source: ShareSource) -> Result<Self, FeatureError> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(FeatureError::Invalid("the URL is empty".to_string()));
        }
        if trimmed.len() > MAX_SHARE_URL_LEN {
            return Err(FeatureError::Invalid(format!(
                "the URL is {} bytes, over the {MAX_SHARE_URL_LEN} a QR code can carry",
                trimmed.len()
            )));
        }

        let rest = trimmed
            .strip_prefix("https://")
            .or_else(|| trimmed.strip_prefix("http://"))
            .ok_or_else(|| {
                FeatureError::Invalid(format!(
                    "{trimmed:?} is not an http or https URL; a tunnel publishes one of those"
                ))
            })?;

        if rest.starts_with("//") {
            return Err(FeatureError::Invalid(format!(
                "{trimmed:?} has an empty authority"
            )));
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("").to_string();
        if authority.is_empty() {
            return Err(FeatureError::Invalid(format!("{trimmed:?} has no host")));
        }
        // Strip any port before looking for credentials, so a bracketed IPv6
        // literal's own colons are not mistaken for a userinfo separator.
        let host = authority
            .rsplit_once(':')
            .filter(|(h, p)| {
                !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && !h.ends_with(']')
            })
            .map_or(authority.clone(), |(h, _)| h.to_string());
        if host.contains('@') {
            return Err(FeatureError::Invalid(format!(
                "{trimmed:?} carries credentials in the host part; sharing it would publish them"
            )));
        }

        Ok(Self {
            url: trimmed.to_string(),
            source,
            note: None,
        })
    }

    /// Build from a provisioned quick tunnel.
    pub fn from_quick_tunnel(
        tunnel: &crate::quicktunnel::QuickTunnel,
    ) -> Result<Self, FeatureError> {
        ShareLink::new(&tunnel.url(), ShareSource::QuickTunnel)
    }

    /// Build from a relay's published handle.
    pub fn from_relay(handle: &crate::relay::TunnelHandle) -> Result<Self, FeatureError> {
        ShareLink::new(&handle.url, ShareSource::Relay)
    }

    /// Attach a note to be shown with the link, such as what it serves.
    ///
    /// The note is escaped wherever it is rendered. It is operator input that
    /// can end up in an HTML page, and an unescaped `<` in a filename would
    /// otherwise become a tag.
    pub fn with_note(mut self, note: &str) -> Self {
        let note = note.trim();
        self.note = if note.is_empty() {
            None
        } else {
            Some(note.to_string())
        };
        self
    }

    /// The validated URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Which mechanism published this URL.
    pub fn source(&self) -> ShareSource {
        self.source
    }

    /// The note, if one was attached.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The host part, for a log line that should not repeat a path.
    pub fn host(&self) -> &str {
        let rest = self
            .url
            .strip_prefix("https://")
            .or_else(|| self.url.strip_prefix("http://"))
            .unwrap_or(&self.url);
        rest.split(['/', '?', '#']).next().unwrap_or(rest)
    }

    /// The QR code for this URL, at the size a terminal can show.
    pub fn qr(&self) -> Result<String, FeatureError> {
        render_qr(&self.url)
    }

    /// A single block of text for a terminal, an issue, or a chat message.
    ///
    /// The URL comes first on its own line because that is what a reader
    /// selects and copies; a QR below it is for the phone.
    pub fn terminal(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.url);
        out.push('\n');
        if let Some(note) = &self.note {
            out.push_str(&format!("// {note}\n"));
        }
        out.push_str(&format!(
            "// shared from podnet ({})\n",
            self.source.label()
        ));
        out
    }

    /// A self-contained HTML page for the link.
    ///
    /// The QR is rendered as half-block characters rather than as an embedded
    /// image so the page has no external reference of any kind: nothing to
    /// fetch, nothing to phone home to, and it survives being saved and
    /// reopened with no network at all.
    pub fn html(&self) -> Result<String, FeatureError> {
        let qr = self.qr()?;
        let note = self
            .note
            .as_deref()
            .map(|n| format!("<p>{}</p>", html_escape(n)));
        Ok(format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>{host}</title>\n\
             <style>\nbody{{font-family:system-ui,sans-serif;margin:2rem auto;max-width:40rem;\
             text-align:center;line-height:1.4}}\npre{{font-family:ui-monospace,monospace;\
             font-size:0.5rem;line-height:1}}\na{{word-break:break-all}}\n\
             </style>\n</head>\n<body>\n<h1>podnet</h1>\n\
             <p>A tunnel published by podnet, {label}.</p>\n{note}\
             <pre>{qr}</pre>\n<p><a href=\"{url}\">{url}</a></p>\n\
             <p>Scan the code, or open the link.</p>\n</body>\n</html>\n",
            host = html_escape(self.host()),
            label = html_escape(self.source.label()),
            note = note.as_deref().unwrap_or(""),
            qr = html_escape(&qr),
            url = html_escape(&self.url),
        ))
    }
}

/// Escape the five characters that can change the meaning of HTML.
pub(super) fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
