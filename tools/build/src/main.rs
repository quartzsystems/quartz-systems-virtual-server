// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS build orchestrator: `cargo run -p build -- <command>`.
//!
//! The pipeline is QS/400's `buildkit`, taken as a library; `product`
//! describes QS/VS to it. What is QS/VS's own is here: the A/B record
//! on the disk image (`image`), and the host checks and the QEMU suite
//! (`host`).

mod host;
mod image;
mod product;

use anyhow::Result;
use buildkit::{Ctx, kernel, notices, userland};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "build", about = "QS/VS build orchestrator", version)]
struct Cli {
    /// Stream tool output instead of writing it to target/logs.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download, verify and extract the pinned kernel tarball.
    Fetch,
    /// Configure and build the kernel.
    Kernel,
    /// Build qsvsinit for the musl target; verify it is static and stripped.
    Userland,
    /// Write the deterministic newc cpio.
    Initramfs,
    /// Rebuild the kernel with the initramfs embedded: the slot A, slot B
    /// and media UKIs.
    Uki,
    /// Write the GPT disk image: two-slot ESP, A/B record, QSVSDATA.
    Disk {
        /// Image size in bytes (default 1 GiB).
        #[arg(long)]
        size: Option<u64>,
    },
    /// Write the UEFI-bootable installation ISO.
    Iso,
    /// Write and sign the artifact manifest.
    Manifest,
    /// Run the whole pipeline.
    All,
    /// Boot under QEMU and check the serial console: `--media disk`
    /// (default; slot A, then slot B) or `iso`.
    Test {
        /// What to boot.
        #[arg(long, default_value = "disk")]
        media: conformance::Media,
    },
    /// Check host prerequisites and the license headers.
    Doctor,
    /// Run cargo-deny plus the C-linkage check of the compiled graph.
    Deny,
    /// Write `NOTICES.txt` (every crate of `Cargo.lock`, its license and
    /// the license texts) into `lic/release/`.
    Notices {
        /// Fail when the file in the tree is stale instead of writing it.
        #[arg(long)]
        check: bool,
    },
}

/// The pipeline in order. The disk step is QS/VS's own, for the record.
fn all(ctx: &Ctx) -> Result<()> {
    tracing::info!("building {} {}", ctx.product.name, ctx.product.full());
    kernel::fetch::fetch(ctx)?;
    kernel::kernel(ctx)?;
    userland::userland(ctx)?;
    buildkit::image::initramfs::initramfs(ctx)?;
    kernel::uki(ctx, true)?;
    image::disk::disk(ctx, None)?;
    buildkit::image::iso::iso(ctx)?;
    buildkit::image::manifest::manifest(ctx)?;
    Ok(())
}

fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let ctx = Ctx::new(product::qsvs()?, cli.verbose)?;
    if !cfg!(target_os = "linux")
        && !matches!(
            cli.command,
            Command::Doctor | Command::Deny | Command::Notices { .. }
        )
    {
        tracing::warn!("the pipeline needs a Linux host; run this from WSL");
    }
    match cli.command {
        Command::Fetch => {
            kernel::fetch::fetch(&ctx)?;
        }
        Command::Kernel => kernel::kernel(&ctx)?,
        Command::Userland => {
            userland::userland(&ctx)?;
        }
        Command::Initramfs => {
            buildkit::image::initramfs::initramfs(&ctx)?;
        }
        Command::Uki => {
            kernel::uki(&ctx, true)?;
        }
        Command::Disk { size } => {
            image::disk::disk(&ctx, size)?;
        }
        Command::Iso => {
            buildkit::image::iso::iso(&ctx)?;
        }
        Command::Manifest => {
            buildkit::image::manifest::manifest(&ctx)?;
        }
        Command::All => all(&ctx)?,
        Command::Test { media } => host::testrun::test(&ctx, media)?,
        Command::Doctor => buildkit::host::doctor::doctor(&ctx)?,
        Command::Deny => buildkit::host::deny::deny(&ctx)?,
        Command::Notices { check } => {
            notices::notices(&ctx, check)?;
        }
    }
    Ok(())
}
