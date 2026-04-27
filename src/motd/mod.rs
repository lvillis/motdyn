mod probe;
mod render;
#[cfg(test)]
mod tests;
mod types;
mod welcome;

use crate::config::MotdConfig;
use probe::collect_snapshot;
use render::{
    build_verbose_items, current_viewer_role, format_aligned_items, paint, render_module_lines,
    resolve_modules, resolve_output_settings,
};
use types::{DEFAULT_FAREWELL, PaintKind};
pub use types::{ModuleProfile, RenderContext};
use welcome::resolve_welcome_text;

pub fn render(verbose: bool, profile: ModuleProfile, cfg: &MotdConfig, ctx: &RenderContext) {
    for line in build_output(verbose, profile, cfg, ctx) {
        println!("{}", line);
    }
}

fn build_output(
    verbose: bool,
    profile: ModuleProfile,
    cfg: &MotdConfig,
    ctx: &RenderContext,
) -> Vec<String> {
    let welcome = resolve_welcome_text(cfg);
    let selection = resolve_modules(cfg, current_viewer_role(), profile);
    let output = resolve_output_settings(cfg);
    let snapshot = collect_snapshot(&selection.modules, cfg);
    let mut lines = Vec::new();

    if !output.compact {
        lines.push(String::new());
    }
    lines.push(welcome.text.clone());
    if !output.compact {
        lines.push(String::new());
    }
    lines.extend(render_module_lines(&selection.modules, &snapshot, &output));

    if verbose {
        if !output.compact {
            lines.push(String::new());
        }
        lines.push(paint("Verbose details:", PaintKind::Header, &output));
        lines.extend(format_aligned_items(
            &build_verbose_items(cfg, ctx, &selection, &welcome, &snapshot, &output),
            &output,
        ));
    }

    if !output.compact {
        lines.push(String::new());
    }
    lines.push(paint(
        resolve_farewell_text(cfg),
        PaintKind::Header,
        &output,
    ));
    lines
}

fn resolve_farewell_text(cfg: &MotdConfig) -> String {
    match cfg.farewell.as_deref() {
        Some(text) if !text.trim().is_empty() => text.to_string(),
        _ => DEFAULT_FAREWELL.to_string(),
    }
}
