use serde::Serialize;

#[derive(Serialize)]
struct ResendEmail<'a> {
    from:    &'a str,
    to:      Vec<&'a str>,
    subject: &'a str,
    html:    String,
}

/// From address untuk semua email keluar.
/// Set env var EMAIL_FROM di Render untuk domain custom yang sudah diverifikasi di Resend.
/// Contoh: "PMPSTI <noreply@pmpsti.ugm.ac.id>"
/// Default: onboarding@resend.dev (hanya berfungsi ke email terverifikasi di Resend)
fn from_address() -> String {
    std::env::var("EMAIL_FROM")
        .unwrap_or_else(|_| "PMPSTI <onboarding@resend.dev>".to_string())
}

pub async fn send_verification_email(
    api_key:    &str,
    to_email:   &str,
    verify_url: &str,
    base_url:   &str,
) -> Result<(), anyhow::Error> {
    let html = format!(r#"
<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: sans-serif; max-width: 600px; margin: 0 auto; padding: 32px;">
  <h2 style="color: #002147;">Verifikasi Email PMPSTI</h2>
  <p>Terima kasih telah mendaftar. Klik tombol di bawah untuk mengaktifkan akun kamu:</p>
  <a href="{verify_url}"
     style="display:inline-block;background:#4f46e5;color:#fff;padding:12px 24px;
            border-radius:8px;text-decoration:none;font-weight:bold;margin:16px 0;">
    Verifikasi Email
  </a>
  <p style="color:#666;font-size:14px;">
    Link ini berlaku selama 24 jam.<br>
    Jika kamu tidak mendaftar, abaikan email ini.
  </p>
  <hr style="border:none;border-top:1px solid #eee;margin-top:32px;">
  <p style="color:#999;font-size:12px;">PMPSTI UGM — {base_url}</p>
</body>
</html>
    "#, verify_url = verify_url, base_url = base_url);

    let from = from_address();
    let client = crate::limits::http_client(std::time::Duration::from_secs(10), std::time::Duration::from_secs(20), Some(std::time::Duration::from_secs(30)));
    let payload = ResendEmail {
        from:    &from,
        to:      vec![to_email],
        subject: "Verifikasi Email — PMPSTI",
        html,
    };

    let res = client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Resend error: {}", body));
    }

    Ok(())
}

pub async fn send_password_reset_email(
    api_key:   &str,
    to_email:  &str,
    reset_url: &str,
    base_url:  &str,
) -> Result<(), anyhow::Error> {
    let html = format!(r#"
<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: sans-serif; max-width: 600px; margin: 0 auto; padding: 32px;">
  <h2 style="color: #002147;">Reset Password PMPSTI</h2>
  <p>Kami menerima permintaan reset password untuk akun ini. Klik tombol di bawah:</p>
  <a href="{reset_url}"
     style="display:inline-block;background:#0055A5;color:#fff;padding:12px 24px;
            border-radius:8px;text-decoration:none;font-weight:bold;margin:16px 0;">
    Reset Password
  </a>
  <p style="color:#666;font-size:14px;">
    Link ini berlaku selama <strong>1 jam</strong>.<br>
    Jika kamu tidak meminta reset password, abaikan email ini.
  </p>
  <hr style="border:none;border-top:1px solid #eee;margin-top:32px;">
  <p style="color:#999;font-size:12px;">PMPSTI UGM — {base_url}</p>
</body>
</html>
    "#, reset_url = reset_url, base_url = base_url);

    let from = from_address();
    let client = crate::limits::http_client(std::time::Duration::from_secs(10), std::time::Duration::from_secs(20), Some(std::time::Duration::from_secs(30)));
    let payload = ResendEmail {
        from:    &from,
        to:      vec![to_email],
        subject: "Reset Password — PMPSTI",
        html,
    };

    let res = client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Resend error: {}", body));
    }

    Ok(())
}

pub async fn send_welcome_email(
    api_key:   &str,
    to_email:  &str,
    reset_url: &str,
    base_url:  &str,
) -> Result<(), anyhow::Error> {
    let html = format!(r#"
<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body style="font-family: sans-serif; max-width: 600px; margin: 0 auto; padding: 32px;">
  <h2 style="color: #002147;">Selamat Datang di PMPSTI</h2>
  <p>Akun kamu telah dibuat oleh administrator. Klik tombol di bawah untuk mengatur password:</p>
  <a href="{reset_url}"
     style="display:inline-block;background:#0055A5;color:#fff;padding:12px 24px;
            border-radius:8px;text-decoration:none;font-weight:bold;margin:16px 0;">
    Atur Password
  </a>
  <p style="color:#666;font-size:14px;">
    Link ini berlaku selama <strong>1 jam</strong>.<br>
    Jika kamu tidak mengenal layanan ini, abaikan email ini.
  </p>
  <hr style="border:none;border-top:1px solid #eee;margin-top:32px;">
  <p style="color:#999;font-size:12px;">PMPSTI UGM — {base_url}</p>
</body>
</html>
    "#, reset_url = reset_url, base_url = base_url);

    let from = from_address();
    let client = crate::limits::http_client(std::time::Duration::from_secs(10), std::time::Duration::from_secs(20), Some(std::time::Duration::from_secs(30)));
    let payload = ResendEmail {
        from:    &from,
        to:      vec![to_email],
        subject: "Selamat Datang — PMPSTI",
        html,
    };

    let res = client
        .post("https://api.resend.com/emails")
        .bearer_auth(api_key)
        .json(&payload)
        .send()
        .await?;

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Resend error: {}", body));
    }

    Ok(())
}
