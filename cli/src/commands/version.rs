use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run() -> Result<()> {
    println!("{}", format!("sentrix v{}", env!("SENTRIX_VERSION")).bold());
    println!(
        "  {:20} {}",
        "Build date:".bright_black(),
        env!("SENTRIX_BUILD_DATE")
    );
    println!(
        "  {:20} {}",
        "Platform:".bright_black(),
        std::env::consts::OS
    );
    println!("  {:20} {}", "Arch:".bright_black(), std::env::consts::ARCH);
    println!(
        "  {:20} {}",
        "Rust:".bright_black(),
        option_env!("CARGO_PKG_RUST_VERSION")
            .unwrap_or("stable")
            .bright_black()
    );
    Ok(())
}
