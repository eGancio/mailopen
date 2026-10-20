//! Server settings for common providers, so most people only type address and password.

use crate::config::SmtpSecurity;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub name: &'static str,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    /// Shown under the password field (app passwords, etc.).
    pub hint: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Gmail",
        imap_host: "imap.gmail.com",
        imap_port: 993,
        smtp_host: "smtp.gmail.com",
        smtp_port: 465,
        smtp_security: SmtpSecurity::Tls,
        hint: "Serve una password per le app (Account Google → Sicurezza → Password per le app) e IMAP attivo.",
    },
    Preset {
        name: "Outlook / Microsoft 365",
        imap_host: "outlook.office365.com",
        imap_port: 993,
        smtp_host: "smtp.office365.com",
        smtp_port: 587,
        smtp_security: SmtpSecurity::StartTls,
        hint: "Gli account aziendali con solo OAuth non funzionano con la password: chiedere all'amministratore.",
    },
    Preset {
        name: "iCloud",
        imap_host: "imap.mail.me.com",
        imap_port: 993,
        smtp_host: "smtp.mail.me.com",
        smtp_port: 587,
        smtp_security: SmtpSecurity::StartTls,
        hint: "Serve una password specifica per le app (appleid.apple.com).",
    },
    Preset {
        name: "Aruba",
        imap_host: "imaps.aruba.it",
        imap_port: 993,
        smtp_host: "smtps.aruba.it",
        smtp_port: 465,
        smtp_security: SmtpSecurity::Tls,
        hint: "",
    },
    Preset {
        name: "Libero",
        imap_host: "imapmail.libero.it",
        imap_port: 993,
        smtp_host: "smtp.libero.it",
        smtp_port: 465,
        smtp_security: SmtpSecurity::Tls,
        hint: "",
    },
    Preset {
        name: "Yahoo",
        imap_host: "imap.mail.yahoo.com",
        imap_port: 993,
        smtp_host: "smtp.mail.yahoo.com",
        smtp_port: 465,
        smtp_security: SmtpSecurity::Tls,
        hint: "Serve una password per le app.",
    },
    Preset {
        name: "Fastmail",
        imap_host: "imap.fastmail.com",
        imap_port: 993,
        smtp_host: "smtp.fastmail.com",
        smtp_port: 465,
        smtp_security: SmtpSecurity::Tls,
        hint: "Serve una password per le app.",
    },
    Preset {
        name: "Altro (IMAP/SMTP manuale)",
        imap_host: "",
        imap_port: 993,
        smtp_host: "",
        smtp_port: 465,
        smtp_security: SmtpSecurity::Tls,
        hint: "",
    },
];

/// Guess a preset from the address domain, falling back to the manual one.
pub fn for_email(email: &str) -> usize {
    let domain = email.rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    let guess = match domain.as_str() {
        "gmail.com" | "googlemail.com" => "Gmail",
        "outlook.com" | "outlook.it" | "hotmail.com" | "hotmail.it" | "live.com" | "live.it"
        | "msn.com" => "Outlook / Microsoft 365",
        "icloud.com" | "me.com" | "mac.com" => "iCloud",
        "aruba.it" | "pec.it" => "Aruba",
        "libero.it" | "virgilio.it" | "iol.it" => "Libero",
        "yahoo.com" | "yahoo.it" | "ymail.com" => "Yahoo",
        "fastmail.com" | "fastmail.fm" => "Fastmail",
        _ => "",
    };
    PRESETS
        .iter()
        .position(|p| p.name == guess)
        .unwrap_or(PRESETS.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_gmail_and_falls_back() {
        assert_eq!(
            PRESETS.get(for_email("x@gmail.com")).map(|p| p.name),
            Some("Gmail")
        );
        assert_eq!(
            PRESETS.get(for_email("x@azienda.it")).map(|p| p.name),
            Some("Altro (IMAP/SMTP manuale)")
        );
        assert_eq!(for_email("senza-chiocciola"), PRESETS.len() - 1);
    }
}
