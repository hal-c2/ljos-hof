//! The fonts a terminal draws with: the family the config names, made
//! fit for a grid on each platform.
//!
//! Two things go wrong when the name is passed through as it is:
//!
//! - On macOS the generic `monospace` stands for Courier, which CoreText
//!   may not list. The text then falls to `sans-serif`, a proportional
//!   face, and every cell is as wide as its "M". Menlo has shipped with
//!   every macOS since 10.6 and is what the system calls fixed pitch.
//! - The symbols prompts draw (powerline arrows, a branch, an icon for a
//!   language) are in Unicode's private use area, which only Nerd Fonts
//!   cover. Fontique falls back by script, and that area has none, so they
//!   are boxes unless a font in the stack has them. Where none does, the
//!   stack ends with a Nerd Font that is installed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use ljos::item::TextEngine;

thread_local! {
    /// The stack for each family named, once: listing the families walks
    /// every font the platform has.
    static STACKS: RefCell<HashMap<Arc<str>, Arc<str>>> = RefCell::default();
}

/// The family list a terminal set in `family` draws with.
pub(crate) fn terminal_family(te: &mut TextEngine, family: &Arc<str>) -> Arc<str> {
    if let Some(known) = STACKS.with(|s| s.borrow().get(family).cloned()) {
        return known;
    }
    let installed = te.family_names();
    let stack: Arc<str> = stack_for(family, &installed, cfg!(target_os = "macos")).into();
    STACKS.with(|s| s.borrow_mut().insert(family.clone(), stack.clone()));
    stack
}

fn stack_for(family: &str, installed: &[String], macos: bool) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for name in family.split(',') {
        let name = name.trim().trim_matches(['"', '\'']);
        if name.is_empty() {
            continue;
        }
        if macos && name.eq_ignore_ascii_case("monospace") {
            stack.push("Menlo");
        }
        stack.push(name);
    }
    if stack.is_empty() {
        stack.push(if macos { "Menlo" } else { "monospace" });
    }
    if !stack.iter().any(|n| is_nerd(n))
        && let Some(symbols) = symbols_family(installed)
    {
        stack.push(symbols);
    }
    stack.join(", ")
}

fn is_nerd(family: &str) -> bool {
    family.to_ascii_lowercase().contains("nerd font")
}

/// The installed Nerd Font best for symbols alone: the symbols-only one,
/// then one whose glyphs are made to fit a cell, then any.
fn symbols_family(installed: &[String]) -> Option<&str> {
    let find = |want: &dyn Fn(&str) -> bool| {
        installed
            .iter()
            .map(String::as_str)
            .find(|n| want(&n.to_ascii_lowercase()))
    };
    find(&|n| n == "symbols nerd font mono")
        .or_else(|| find(&|n| n == "symbols nerd font"))
        .or_else(|| find(&|n| n.contains("nerd font mono")))
        .or_else(|| find(&|n| n.contains("nerd font")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn on_macos_monospace_is_menlo_first() {
        assert_eq!(stack_for("monospace", &[], true), "Menlo, monospace");
        assert_eq!(stack_for("monospace", &[], false), "monospace");
        assert_eq!(stack_for("", &[], true), "Menlo");
        assert_eq!(
            stack_for("Iosevka, monospace", &[], true),
            "Iosevka, Menlo, monospace"
        );
    }

    #[test]
    fn a_stack_without_a_nerd_font_ends_with_one_installed() {
        let installed = names(&[
            "Arial",
            "CaskaydiaMono Nerd Font",
            "Hack Nerd Font Mono",
            "Menlo",
        ]);
        assert_eq!(
            stack_for("monospace", &installed, true),
            "Menlo, monospace, Hack Nerd Font Mono"
        );
        let installed = names(&["Hack Nerd Font Mono", "Symbols Nerd Font Mono"]);
        assert_eq!(
            stack_for("Menlo", &installed, true),
            "Menlo, Symbols Nerd Font Mono"
        );
    }

    #[test]
    fn a_stack_with_a_nerd_font_is_left_as_it_is() {
        let installed = names(&["Symbols Nerd Font Mono"]);
        assert_eq!(
            stack_for("JetBrainsMono Nerd Font", &installed, false),
            "JetBrainsMono Nerd Font"
        );
    }
}
