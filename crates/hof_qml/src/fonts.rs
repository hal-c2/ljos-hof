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
//!
//! Both belong in Ljós UI's text engine, and go from here once it has them:
//! hal-c2/ljos-ui#189 and hal-c2/ljos-ui#190.
//!
//! The stack goes to the text engine as a first family and the families
//! after it, never as one string with commas: Ljós UI reads a family as one
//! name whatever it holds, as Qt does, and "Menlo, monospace" names no font.
//! Text then falls to a proportional face, and every cell is as wide as its
//! "M" again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use ljos::item::{TextEngine, TextStyle};

/// Families, the first tried first.
type Stack = Arc<[Arc<str>]>;

thread_local! {
    /// The stack for each family named, once: listing the families walks
    /// every font the platform has.
    static STACKS: RefCell<HashMap<Arc<str>, Stack>> = RefCell::default();
}

/// The installed families whose letters are all one width, sorted: the
/// ones a terminal's grid fits, to choose from in the settings. Found on a
/// thread of its own with a text engine of its own, since it reads a face
/// of every family, and given to `done` there.
pub fn find_fixed_pitch_families(done: impl FnOnce(Vec<String>) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("hof-fonts".into())
        .spawn(move || done(fixed_pitch_in(&mut TextEngine::with_system_fonts())));
    if let Err(e) = spawned {
        eprintln!("hof: cannot list the fonts: {e}");
    }
}

fn fixed_pitch_in(te: &mut TextEngine) -> Vec<String> {
    te.family_names()
        .into_iter()
        .filter(|name| {
            let style = TextStyle {
                family: name.as_str().into(),
                pixel_size: 16.0,
                ..TextStyle::default()
            };
            let face = te.face_info(&style);
            // A family the engine set in another's face is not one to name.
            if !face.fixed_pitch || !face.family.eq_ignore_ascii_case(name) {
                return false;
            }
            // Its letters too, not a face of emoji or icons all one width
            // whose letters come from another.
            let (m, i) = (te.advance_width("M", &style), te.advance_width("i", &style));
            m > 0.0 && (m - i).abs() < 0.01
        })
        .collect()
}

/// Makes `style` draw with the stack a terminal set in its family draws
/// with: the first in `family`, the rest before any it already falls back
/// to.
pub(crate) fn terminal_family(te: &mut TextEngine, style: &mut TextStyle) {
    let stack = STACKS.with(|s| s.borrow().get(&style.family).cloned());
    let stack = stack.unwrap_or_else(|| {
        let installed = te.family_names();
        let stack: Stack = stack_for(&style.family, &installed, cfg!(target_os = "macos"))
            .into_iter()
            .map(Arc::from)
            .collect();
        STACKS.with(|s| s.borrow_mut().insert(style.family.clone(), stack.clone()));
        stack
    });
    set_stack(style, &stack);
}

fn set_stack(style: &mut TextStyle, stack: &[Arc<str>]) {
    let Some((first, rest)) = stack.split_first() else {
        return;
    };
    let mut fallback: Vec<Arc<str>> = rest.to_vec();
    fallback.extend(
        style
            .fallback_families
            .iter()
            .flat_map(|f| f.iter().cloned()),
    );
    style.family = first.clone();
    style.fallback_families = (!fallback.is_empty()).then(|| Arc::new(fallback));
}

fn stack_for(family: &str, installed: &[String], macos: bool) -> Vec<String> {
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
    stack.into_iter().map(str::to_owned).collect()
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

    /// `stack_for`, written as a list for short asserts.
    fn stack(family: &str, installed: &[String], macos: bool) -> String {
        stack_for(family, installed, macos).join(", ")
    }

    #[test]
    fn the_stack_is_a_family_and_its_fallbacks_not_one_name_with_commas() {
        let mut style = TextStyle {
            family: "monospace".into(),
            fallback_families: Some(Arc::new(vec!["Noto Sans Mono".into()])),
            ..TextStyle::default()
        };
        let stack = names(&["Menlo", "monospace", "Hack Nerd Font Mono"]);
        set_stack(
            &mut style,
            &stack.into_iter().map(Arc::from).collect::<Vec<_>>(),
        );
        assert_eq!(&*style.family, "Menlo");
        let fallback: Vec<&str> = style
            .fallback_families
            .iter()
            .flat_map(|f| f.iter().map(|n| &**n))
            .collect();
        assert_eq!(
            fallback,
            ["monospace", "Hack Nerd Font Mono", "Noto Sans Mono"]
        );

        let mut alone = TextStyle {
            family: "Iosevka".into(),
            ..TextStyle::default()
        };
        set_stack(&mut alone, &[Arc::from("Iosevka")]);
        assert_eq!(&*alone.family, "Iosevka");
        assert!(alone.fallback_families.is_none());
    }

    #[test]
    fn on_macos_monospace_is_menlo_first() {
        assert_eq!(stack("monospace", &[], true), "Menlo, monospace");
        assert_eq!(stack("monospace", &[], false), "monospace");
        assert_eq!(stack("", &[], true), "Menlo");
        assert_eq!(
            stack("Iosevka, monospace", &[], true),
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
            stack("monospace", &installed, true),
            "Menlo, monospace, Hack Nerd Font Mono"
        );
        let installed = names(&["Hack Nerd Font Mono", "Symbols Nerd Font Mono"]);
        assert_eq!(
            stack("Menlo", &installed, true),
            "Menlo, Symbols Nerd Font Mono"
        );
    }

    #[test]
    fn a_stack_with_a_nerd_font_is_left_as_it_is() {
        let installed = names(&["Symbols Nerd Font Mono"]);
        assert_eq!(
            stack("JetBrainsMono Nerd Font", &installed, false),
            "JetBrainsMono Nerd Font"
        );
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    /// Through the platform's own fonts: whatever the stack holds, the
    /// face a terminal draws with is fixed pitch, so a cell is as wide as
    /// any letter in it.
    ///
    /// The widths tell only on macOS: on Linux fontconfig finds a
    /// monospace face even for "Menlo, monospace" taken as one name, so
    /// the style is checked to hold the stack, each family its own name,
    /// wherever the test runs.
    #[test]
    fn a_terminal_draws_with_a_fixed_pitch_face() {
        let mut te = TextEngine::with_system_fonts();
        let installed = te.family_names();
        for family in ["monospace", "Menlo, monospace", "No Such Font, monospace"] {
            let mut style = TextStyle {
                family: family.into(),
                pixel_size: 16.0,
                ..TextStyle::default()
            };
            terminal_family(&mut te, &mut style);
            let drawn: Vec<&str> = std::iter::once(&*style.family)
                .chain(
                    style
                        .fallback_families
                        .iter()
                        .flat_map(|f| f.iter().map(|n| &**n)),
                )
                .collect();
            assert_eq!(
                drawn,
                stack_for(family, &installed, cfg!(target_os = "macos")),
                "{family}"
            );
            let m = te.advance_width("M", &style);
            let i = te.advance_width("i", &style);
            assert!(
                (m - i).abs() < 0.01,
                "{family} draws with a proportional face: M {m}, i {i}"
            );
        }
    }

    /// Each family offered is fixed pitch, and the proportional faces every
    /// system has are not.
    #[test]
    fn the_families_offered_are_fixed_pitch() {
        let mut te = TextEngine::with_system_fonts();
        let fixed = fixed_pitch_in(&mut te);
        assert!(!fixed.is_empty());
        for name in &fixed {
            let style = TextStyle {
                family: name.as_str().into(),
                pixel_size: 16.0,
                ..TextStyle::default()
            };
            let (m, i) = (te.advance_width("M", &style), te.advance_width("i", &style));
            assert!((m - i).abs() < 0.01, "{name}: M {m}, i {i}");
        }
        let installed = te.family_names();
        for proportional in ["DejaVu Sans", "Noto Sans", "Helvetica", "Arial"] {
            if installed.iter().any(|n| n == proportional) {
                assert!(!fixed.iter().any(|n| n == proportional), "{proportional}");
            }
        }
    }
}
