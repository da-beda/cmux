pub mod app;
pub mod ghostty_actions;
pub mod model;
pub mod notifications;
pub mod persistence;
pub mod socket;
pub mod ui;

use tracing_subscriber::EnvFilter;

pub fn run() -> i32 {
    prefer_desktop_opengl();
    init_logging();

    tracing::info!("cmux starting");
    app::run()
}

fn init_logging() {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn prefer_desktop_opengl() {
    const FLAG: &str = "gl-prefer-gl";
    match std::env::var("GDK_DEBUG") {
        Ok(existing) if existing.split(',').any(|flag| flag.trim() == FLAG) => {}
        Ok(existing) if existing.trim().is_empty() => std::env::set_var("GDK_DEBUG", FLAG),
        Ok(existing) => std::env::set_var("GDK_DEBUG", format!("{existing},{FLAG}")),
        Err(_) => std::env::set_var("GDK_DEBUG", FLAG),
    }
}
