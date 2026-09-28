//! `web.mail` module — Send emails via SMTP.
//!
//! ```text
//! use web.mail
//!
//! mailer = smtp['smtp.example.com', 587, 'noreply@example.com', 'pass']
//! fn welcome[m, to]
//!     mail[m, to, 'Hello', 'Welcome to Crow!']
//! ```
//!
//! In a handler `mail` waits for the commit, and a handler that stops sends
//! nothing (`crow_web::live`, `Held::Mail`).

/// SMTP mailer wrapper around lettre. Its messages come from its user.
#[derive(Clone)]
pub struct CrowMailer {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from: String,
    host: String,
}

/// A mailer shows as it displays (`<mailer:host>`), through the one
/// display `log` and `str` use in both modes.
impl crow_runtime::display::CrowDisplay for CrowMailer {
    fn crow_repr(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for CrowMailer {
    /// A mailer shows its host, never its password.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "<mailer:{}>", self.host)
    }
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

        Ok(Self { transport, from: username.to_string(), host: host.to_string() })
    }

    /// `mail[mailer, to, subject, body]`: a plain-text message from the
    /// mailer's user, sent from where no runtime waits — a handler's worker
    /// thread, the interpreter — on a thread of its own. Under
    /// `CROW_TEST_CONTROL` (`crow test --compiled`) nothing is sent.
    pub fn send_blocking(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        if std::env::var("CROW_TEST_CONTROL").is_ok() {
            return Ok(());
        }
        let (mailer, to, subject, body) = (self.clone(), to.to_string(), subject.to_string(), body.to_string());
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            rt.block_on(mailer.send(&mailer.from, &to, &subject, &body))
        })
        .join()
        .map_err(|_| "the mail thread failed".to_string())?
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

