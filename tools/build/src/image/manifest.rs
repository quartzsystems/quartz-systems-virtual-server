// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build manifest`: `buildkit` writes and signs the manifest; this
//! reads it back and holds it to the kind of image it is of. A release
//! image's manifest names the product and lists no test hooks; a test
//! image's says so in its `product` and lists `qsvshook`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use buildkit::Ctx;
use tracing::info;

use super::userland::check_hooks;
use crate::product::{HOOK, Image, TEST_NAME};

/// What the manifest says of its image: the product, and whether the
/// test hooks' binary is among the artifacts.
fn kind_of(manifest: &str) -> Result<(String, bool)> {
    let json: serde_json::Value = serde_json::from_str(manifest)?;
    let product = json["product"]
        .as_str()
        .context("the manifest has no product")?
        .to_string();
    let hooks = json["artifacts"]
        .as_array()
        .context("the manifest has no artifacts")?
        .iter()
        .any(|a| a["kind"] == HOOK);
    Ok((product, hooks))
}

/// `build manifest`.
pub fn manifest(ctx: &Ctx, image: Image) -> Result<PathBuf> {
    check_hooks(ctx, image)?;
    let path = buildkit::image::manifest::manifest(ctx)?;
    let (product, hooks) = kind_of(&fs::read_to_string(&path)?)?;
    let expected = match image {
        Image::Release => (release::NAME, false),
        Image::Test => (TEST_NAME, true),
    };
    ensure!(
        (product.as_str(), hooks) == expected,
        "the manifest is of `{product}`, test hooks listed: {hooks}"
    );
    info!("manifest: product `{product}`, test hooks listed: {hooks}");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_says_which_kind_of_image_it_is_of() {
        let release =
            r#"{"product": "QS/VS", "artifacts": [{"kind": "qsvsinit"}, {"kind": "disk"}]}"#;
        assert_eq!(kind_of(release).unwrap(), ("QS/VS".into(), false));
        let test = r#"{"product": "QS/VS (test image)", "artifacts": [{"kind": "qsvshook"}]}"#;
        assert_eq!(kind_of(test).unwrap(), (TEST_NAME.into(), true));
        assert!(kind_of("{}").is_err());
    }
}
