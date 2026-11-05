//! Writes the setup pages to ~/Downloads so their look can be checked by eye.
//! Run with `cargo run --example preview_pages`.

use messaging_mcp::setup::qr::{Tone, encode, page, status_page, svg};

fn main() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = std::path::Path::new(&home).join("Downloads");
    let qr = encode("tg://login?token=preview-only")
        .map(|matrix| svg(&matrix))
        .unwrap_or_default();
    let steps = [
        "Open <b>Telegram</b> on your phone.",
        "Go to <b>Settings</b>, then <b>Devices</b>.",
        "Tap <b>Link Desktop Device</b> and point it here.",
    ];
    let pages = [
        ("preview-qr.html", page("Scan with Telegram", &steps, &qr)),
        (
            "preview-success.html",
            status_page(
                Tone::Success,
                "You're connected",
                "Telegram is linked to Messaging MCP. You can close this tab.",
            ),
        ),
        (
            "preview-notice.html",
            status_page(
                Tone::Notice,
                "One more step",
                "Your Telegram account has a 2FA password. Type it in a terminal with \
                 messaging-mcp telegram-password.",
            ),
        ),
        (
            "preview-failure.html",
            status_page(
                Tone::Failure,
                "Telegram login stopped",
                "Telegram refused the login request.",
            ),
        ),
    ];
    for (name, html) in pages {
        let _ = std::fs::write(dir.join(name), html);
    }
}
