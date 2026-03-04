//! `web.mail` module — Send emails via SMTP.
//!
//! ```text
//! use web.mail
//!
//! mailer = smtp['smtp.example.com', 587, 'user', 'pass']
//! send[mailer, {
//!   from, 'noreply@example.com',
//!   to, 'alice@example.com',
//!   subject, 'Hello',
//!   body, 'Welcome to Crow!'
//! }]
//! ```

/// SMTP mailer wrapper around lettre.
pub struct CrowMailer {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
}

impl CrowMailer {
    /// Connect to an SMTP server with STARTTLS.
    pub fn new(host: &str, port: u16, username: &str, password: &str) -> Result<Self, String> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::AsyncSmtpTransport;

        let creds = Credentials::new(username.to_string(), password.to_string());
        let transport = AsyncSmtpTransport::<lettre::Tokio1Executor>::starttls_relay(host)
            .map_err(|e| e.to_string())?
            .port(port)
            .credentials(creds)
            .build();

        Ok(Self { transport })
    }

    /// Send an email.
    pub async fn send(
        &self,
        from: &str,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), String> {
        use lettre::message::header::ContentType;
        use lettre::{AsyncTransport, Message};

        let email = Message::builder()
            .from(from.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
            .to(to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| e.to_string())?;

        self.transport
            .send(email)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Send an HTML email.
    pub async fn send_html(
        &self,
        from: &str,
        to: &str,
        subject: &str,
        html: &str,
    ) -> Result<(), String> {
        use lettre::message::header::ContentType;
        use lettre::{AsyncTransport, Message};

        let email = Message::builder()
            .from(from.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
            .to(to.parse().map_err(|e: lettre::address::AddressError| e.to_string())?)
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(html.to_string())
            .map_err(|e| e.to_string())?;

        self.transport
            .send(email)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }
}

/// A simple email message (for building without a mailer).
#[derive(Debug, Clone)]
pub struct CrowEmail {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    pub html: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crow_email_struct() {
        let email = CrowEmail {
            from: "a@b.com".to_string(),
            to: "c@d.com".to_string(),
            subject: "Test".to_string(),
            body: "Hello".to_string(),
            html: false,
        };
        assert_eq!(email.from, "a@b.com");
        assert_eq!(email.to, "c@d.com");
        assert!(!email.html);
    }

    #[test]
    fn test_crow_email_html() {
        let email = CrowEmail {
            from: "a@b.com".to_string(),
            to: "c@d.com".to_string(),
            subject: "Test".to_string(),
            body: "<h1>Hi</h1>".to_string(),
            html: true,
        };
        assert!(email.html);
        assert!(email.body.contains("<h1>"));
    }
}

