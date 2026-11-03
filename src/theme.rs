//! Colours, fonts and the VS Code "Dark+"-style look.

use egui::text::{ByteIndex, LayoutJob};
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, vec2};
use egui_extras::syntax_highlighting::{CodeTheme, SyntectSettings, highlight_with};
use std::sync::{Arc, Mutex, OnceLock};
use syntect::highlighting::{Color, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSet, ThemeSettings};

pub const ACCENT: Color32 = Color32::from_rgb(0x9b, 0x7b, 0xff);
pub const WARM: Color32 = Color32::from_rgb(0xff, 0xc8, 0x57);
pub const OK: Color32 = Color32::from_rgb(0x5e, 0xe0, 0x9a);
pub const DANGER: Color32 = Color32::from_rgb(0xd9, 0x4a, 0x5a);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xb4, 0xae, 0xc8);
pub const LIVE: Color32 = Color32::from_rgb(0x3d, 0xd6, 0xff);

pub const CODE_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
pub const USER_BUBBLE: Color32 = Color32::from_rgb(0x2b, 0x33, 0x40);
pub const AGENT_BUBBLE: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2c);

/// VS Code Dark+ workbench colours.
pub mod vs {
    use egui::Color32;
    pub const TITLE_BG: Color32 = Color32::from_rgb(0x3c, 0x3c, 0x3c);
    pub const ACTIVITY_BG: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
    pub const ACTIVITY_FG: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
    pub const ACTIVITY_DIM: Color32 = Color32::from_rgb(0x85, 0x85, 0x85);
    pub const SIDEBAR_BG: Color32 = Color32::from_rgb(0x25, 0x25, 0x26);
    pub const SIDEBAR_TITLE: Color32 = Color32::from_rgb(0xbb, 0xbb, 0xbb);
    pub const LIST_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x2e);
    pub const LIST_ACTIVE: Color32 = Color32::from_rgb(0x37, 0x37, 0x3d);
    pub const LIST_FOCUS: Color32 = Color32::from_rgb(0x04, 0x39, 0x5e);
    pub const EDITOR_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    pub const TEXT: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xcc);
    pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
    pub const LINE_NO: Color32 = Color32::from_rgb(0x85, 0x85, 0x85);
    pub const LINE_NO_ACTIVE: Color32 = Color32::from_rgb(0xc6, 0xc6, 0xc6);
    pub const LINE_HL: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
    pub const INDENT_GUIDE: Color32 = Color32::from_rgb(0x40, 0x40, 0x40);
    pub const TAB_BAR_BG: Color32 = Color32::from_rgb(0x25, 0x25, 0x26);
    pub const TAB_BG: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x2d);
    pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    pub const TAB_FG: Color32 = Color32::from_rgb(0x96, 0x96, 0x96);
    pub const TAB_BORDER: Color32 = Color32::from_rgb(0x25, 0x25, 0x26);
    pub const FOCUS_BLUE: Color32 = Color32::from_rgb(0x00, 0x78, 0xd4);
    pub const STATUS_BG: Color32 = Color32::from_rgb(0x00, 0x7a, 0xcc);
    pub const STATUS_BUSY: Color32 = Color32::from_rgb(0xcc, 0x66, 0x33);
    pub const BREADCRUMB: Color32 = Color32::from_rgb(0xa9, 0xa9, 0xa9);
    pub const INPUT_BG: Color32 = Color32::from_rgb(0x3c, 0x3c, 0x3c);
    pub const SELECTION: Color32 = Color32::from_rgb(0x26, 0x4f, 0x78);
    pub const MATCH: Color32 = Color32::from_rgb(0x62, 0x33, 0x15);
}

pub fn apply(ctx: &egui::Context) {
    load_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        let v = &mut s.visuals;
        v.panel_fill = vs::SIDEBAR_BG;
        v.window_fill = Color32::from_rgb(0x25, 0x25, 0x26);
        v.window_stroke = egui::Stroke::new(1.0, Color32::from_rgb(0x45, 0x45, 0x45));
        v.extreme_bg_color = vs::INPUT_BG;
        v.faint_bg_color = Color32::from_rgb(0x2a, 0x2a, 0x2b);
        v.code_bg_color = CODE_BG;
        v.selection.bg_fill = vs::SELECTION;
        v.hyperlink_color = Color32::from_rgb(0x37, 0x94, 0xff);
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(4);
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(8.0, 4.0);
    });
}

/// Use the fonts VS Code uses on Windows when they are installed:
/// Segoe UI for the workbench and Consolas for code. egui's bundled fonts stay
/// in the fallback chain for emoji and symbols.
fn load_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    let mut add = |name: &str, file: &str, family: FontFamily| {
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            fonts.font_data.insert(name.to_string(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.entry(family).or_default().insert(0, name.to_string());
        }
    };
    add("segoe", "segoeui.ttf", FontFamily::Proportional);
    add("consolas", "consola.ttf", FontFamily::Monospace);
    ctx.set_fonts(fonts);
}

fn c(hex: u32) -> Color {
    Color {
        r: (hex >> 16) as u8,
        g: (hex >> 8) as u8,
        b: hex as u8,
        a: 0xff,
    }
}

/// Dark+ token colours as a syntect theme: VS Code's `dark_vs` + `dark_plus`
/// rules, plus the scope names Sublime grammars use for the same things
/// (e.g. `variable.function` for calls, `support.macro` for Rust macros).
fn dark_plus() -> Theme {
    let rules: &[(&str, u32)] = &[
        // dark_vs
        ("meta.embedded, source.groovy.embedded, meta.template.expression", 0xd4d4d4),
        ("comment, punctuation.definition.comment", 0x6a9955),
        ("constant.language", 0x569cd6),
        (
            "constant.numeric, constant.numeric.suffix, variable.other.enummember, keyword.operator.plus.exponent, keyword.operator.minus.exponent",
            0xb5cea8,
        ),
        ("constant.regexp", 0x646695),
        ("entity.name.tag", 0x569cd6),
        ("entity.name.tag.css, entity.name.tag.less, entity.name.tag.scss", 0xd7ba7d),
        ("entity.other.attribute-name", 0x9cdcfe),
        (
            "entity.other.attribute-name.class.css, entity.other.attribute-name.class.mixin.css, entity.other.attribute-name.id.css, \
             entity.other.attribute-name.parent-selector.css, entity.other.attribute-name.pseudo-class.css, \
             entity.other.attribute-name.pseudo-element.css, entity.other.attribute-name.scss, entity.other.pseudo-class.css, \
             entity.other.pseudo-element.css, punctuation.definition.entity.css, entity.other.attribute-name.class.scss, \
             entity.other.attribute-name.id.scss, entity.other.attribute-name.less",
            0xd7ba7d,
        ),
        ("invalid, invalid.illegal", 0xf44747),
        ("markup.bold, markup.heading, entity.name.section, punctuation.definition.heading", 0x569cd6),
        ("markup.inserted", 0xb5cea8),
        ("markup.deleted", 0xce9178),
        ("markup.changed", 0x569cd6),
        ("markup.quote, punctuation.definition.quote.begin.markdown, punctuation.definition.blockquote.markdown", 0x6a9955),
        ("punctuation.definition.list.begin.markdown, punctuation.definition.list_item.markdown, markup.list.numbered.bullet", 0x6796e6),
        ("markup.inline.raw, markup.raw, markup.raw.inline, markup.raw.block", 0xce9178),
        ("markup.underline.link, meta.link.inline.markdown string.other.link, markup.underline.link.markdown", 0x3794ff),
        ("punctuation.definition.tag", 0x808080),
        ("meta.preprocessor, entity.name.function.preprocessor", 0x569cd6),
        ("meta.preprocessor.string", 0xce9178),
        ("meta.preprocessor.numeric", 0xb5cea8),
        ("meta.structure.dictionary.key.python", 0x9cdcfe),
        ("meta.diff.header", 0x569cd6),
        ("storage, storage.type, storage.modifier, keyword.operator.noexcept", 0x569cd6),
        ("string, meta.embedded.assembly, string.tag, string.value, punctuation.definition.string", 0xce9178),
        ("string.regexp", 0xd16969),
        (
            "punctuation.definition.template-expression.begin, punctuation.definition.template-expression.end, \
             punctuation.section.embedded, punctuation.section.interpolation, punctuation.definition.interpolation",
            0x569cd6,
        ),
        ("meta.template.expression, meta.interpolation", 0xd4d4d4),
        (
            "support.type.vendored.property-name, support.type.property-name, source.css variable, source.coffee.embedded, \
             meta.property-name.css, support.type.custom-property",
            0x9cdcfe,
        ),
        ("keyword", 0x569cd6),
        ("keyword.operator, punctuation.accessor, punctuation.separator, punctuation.terminator", 0xd4d4d4),
        (
            "keyword.operator.new, keyword.operator.expression, keyword.operator.cast, keyword.operator.sizeof, \
             keyword.operator.alignof, keyword.operator.typeid, keyword.operator.alignas, keyword.operator.instanceof, \
             keyword.operator.logical.python, keyword.operator.wordlike, keyword.operator.word, keyword.other.new, \
             keyword.operator.type",
            0x569cd6,
        ),
        ("keyword.other.unit, keyword.other.unit.css", 0xb5cea8),
        ("punctuation.section.embedded.begin.php, punctuation.section.embedded.end.php", 0x569cd6),
        ("support.function.git-rebase", 0x9cdcfe),
        ("constant.sha.git-rebase", 0xb5cea8),
        ("storage.modifier.import.java, variable.language.wildcard.java, storage.modifier.package.java", 0xd4d4d4),
        ("variable.language", 0x569cd6),
        // dark_plus
        (
            "entity.name.function, support.function, support.constant.handlebars, source.powershell variable.other.member, \
             entity.name.operator.custom-literal, variable.function, meta.function-call.identifier, support.macro, \
             entity.name.function.macro, meta.annotation.identifier, entity.name.function.decorator, \
             variable.annotation, keyword.operator.or.regexp, keyword.control.anchor.regexp",
            0xdcdcaa,
        ),
        (
            "support.class, support.type, entity.name.type, entity.name.namespace, entity.other.attribute, \
             entity.name.scope-resolution, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, \
             entity.name.impl, entity.name.interface, entity.name.union, storage.type.numeric.go, storage.type.byte.go, \
             storage.type.boolean.go, storage.type.string.go, storage.type.uintptr.go, storage.type.error.go, \
             storage.type.rune.go, storage.type.cs, storage.type.generic.cs, storage.type.modifier.cs, \
             storage.type.variable.cs, storage.type.annotation.java, storage.type.generic.java, storage.type.java, \
             storage.type.object.array.java, storage.type.primitive.array.java, storage.type.primitive.java, \
             storage.type.token.java, storage.type.groovy, storage.type.annotation.groovy, storage.type.parameters.groovy, \
             storage.type.generic.groovy, storage.type.object.array.groovy, storage.type.primitive.array.groovy, \
             storage.type.primitive.groovy, storage.type.primitive.dart, storage.type.annotation, \
             punctuation.definition.annotation, meta.type.cast.expr, meta.type.new.expr, support.constant.math, \
             support.constant.dom, support.constant.json, entity.other.inherited-class, punctuation.separator.namespace.ruby, \
             support.type.object.console, support.class.builtin",
            0x4ec9b0,
        ),
        (
            "keyword.control, source.cpp keyword.operator.new, keyword.operator.delete, keyword.other.using, \
             keyword.other.directive.using, keyword.other.operator, entity.name.operator, keyword.control.import, \
             keyword.control.from, keyword.control.export, keyword.control.flow, keyword.control.conditional, \
             keyword.control.loop, keyword.control.return, keyword.control.at-rule, keyword.control.directive, \
             keyword.other.import, keyword.other.use, keyword.control.php",
            0xc586c0,
        ),
        (
            "variable, meta.definition.variable.name, support.variable, entity.name.variable, constant.other.placeholder, \
             variable.parameter, variable.other.readwrite, variable.other.member, variable.other.property, \
             variable.other.object, variable.other.php, punctuation.definition.variable.php, meta.object-literal.key, \
             meta.mapping.key string, support.type.property-name.json, meta.structure.dictionary.key.json string",
            0x9cdcfe,
        ),
        ("variable.other.constant, variable.other.enummember, constant.other.caps, constant.other", 0x4fc1ff),
        (
            "support.constant.property-value, support.constant.font-name, support.constant.media-type, \
             support.constant.media, constant.other.color.rgb-value, constant.other.rgb-value, support.constant.color, \
             constant.other.color, meta.property-value.css support.constant",
            0xce9178,
        ),
        (
            "punctuation.definition.group.regexp, punctuation.definition.group.assertion.regexp, \
             punctuation.definition.character-class.regexp, punctuation.character.set.begin.regexp, \
             punctuation.character.set.end.regexp, keyword.operator.negation.regexp, support.other.parenthesis.regexp",
            0xce9178,
        ),
        (
            "constant.character.character-class.regexp, constant.other.character-class.set.regexp, \
             constant.other.character-class.regexp, constant.character.set.regexp",
            0xd16969,
        ),
        ("keyword.operator.quantifier.regexp", 0xd7ba7d),
        ("constant.character, constant.other.option", 0x569cd6),
        ("constant.character.escape", 0xd7ba7d),
        ("entity.name.label", 0xc8c8c8),
        // Data formats: YAML/TOML/INI keys look like VS Code's.
        ("entity.name.tag.yaml, meta.mapping.key.yaml string", 0x569cd6),
        ("meta.tag.table.toml, entity.name.section.toml, meta.section.ini, entity.name.section.group-title.ini", 0x569cd6),
        ("meta.tag.key.toml, entity.name.tag.toml, keyword.other.definition.ini", 0x9cdcfe),
        ("support.function.builtin.shell, support.function.builtin.bash", 0xdcdcaa),
    ];
    Theme {
        name: Some("Dark+".into()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(c(0xd4d4d4)),
            background: Some(c(0x1e1e1e)),
            caret: Some(c(0xaeafad)),
            line_highlight: Some(c(0x282828)),
            selection: Some(c(0x264f78)),
            ..Default::default()
        },
        scopes: rules
            .iter()
            .filter_map(|(sel, col)| {
                Some(ThemeItem {
                    scope: sel.parse::<ScopeSelectors>().ok()?,
                    style: StyleModifier {
                        foreground: Some(c(*col)),
                        background: None,
                        font_style: None,
                    },
                })
            })
            .collect(),
    }
}

/// Syntect settings: bat's extended grammar set (Dart, TypeScript, TOML,
/// Kotlin, Swift, Vue, SCSS, Dockerfile, …) and Dark+ in the default dark
/// theme slot. Static so the highlighter cache (keyed by address) stays warm.
pub fn syntect() -> &'static SyntectSettings {
    static S: OnceLock<SyntectSettings> = OnceLock::new();
    S.get_or_init(|| {
        let mut ts = ThemeSet::load_defaults();
        ts.themes.insert("base16-mocha.dark".into(), dark_plus());
        SyntectSettings {
            ps: two_face::syntax::extra_newlines(),
            ts,
        }
    })
}

/// Name of the grammar to use for a file (also shown in the status bar).
pub fn syntax_name(file: &str) -> String {
    let ps = &syntect().ps;
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let by_ext = |e: &str| if e.is_empty() { None } else { ps.find_syntax_by_extension(e) };
    let alias = match ext {
        "mjs" | "cjs" => "js",
        "mts" | "cts" => "ts",
        "htm" | "xhtml" => "html",
        "yml" => "yaml",
        "h" => "c",
        "hh" | "hxx" => "hpp",
        "pyw" => "py",
        "kts" => "kt",
        _ => ext,
    };
    by_ext(alias)
        .or_else(|| by_ext(name))
        .or_else(|| by_ext(&lower))
        .or_else(|| match lower.as_str() {
            n if n.starts_with(".env") => ps.find_syntax_by_name("DotENV").or_else(|| by_ext("sh")),
            "dockerfile" | "containerfile" => ps.find_syntax_by_name("Dockerfile"),
            "makefile" | "gnumakefile" => ps.find_syntax_by_name("Makefile"),
            _ => None,
        })
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "Plain Text".into())
}

/// VS Code's bracket pair colours, cycling by nesting depth.
const BRACKETS: [Color32; 3] = [
    Color32::from_rgb(0xff, 0xd7, 0x00),
    Color32::from_rgb(0xda, 0x70, 0xd6),
    Color32::from_rgb(0x17, 0x9f, 0xff),
];
const UNMATCHED: Color32 = Color32::from_rgb(0xff, 0x12, 0x12);

/// Text inside strings, comments and regexes keeps its colour.
fn is_literal(c: Color32) -> bool {
    [0xce9178u32, 0x6a9955, 0xd16969, 0xd7ba7d, 0x3794ff]
        .iter()
        .any(|h| c == Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, *h as u8))
}

/// Recolour (), [] and {} by depth, like VS Code's bracket pair colorization.
pub fn colorize_brackets(job: &LayoutJob) -> LayoutJob {
    let mut out = job.clone();
    out.sections.clear();
    let text = job.text.as_str();
    let mut depth = 0usize;
    for sec in &job.sections {
        let (start, end) = (sec.byte_range.start.0, sec.byte_range.end.0);
        if is_literal(sec.format.color) {
            out.sections.push(sec.clone());
            continue;
        }
        let mut push = |a: usize, b: usize, color: Option<Color32>, lead: f32| {
            if a < b {
                let mut s = sec.clone();
                s.byte_range = ByteIndex(a)..ByteIndex(b);
                s.leading_space = lead;
                if let Some(c) = color {
                    s.format.color = c;
                }
                out.sections.push(s);
            }
        };
        let mut last = start;
        let mut lead = sec.leading_space;
        for (i, ch) in text.get(start..end).unwrap_or("").char_indices() {
            let at = start + i;
            let color = match ch {
                '(' | '[' | '{' => {
                    let c = BRACKETS[depth % 3];
                    depth += 1;
                    c
                }
                ')' | ']' | '}' => {
                    if depth == 0 {
                        UNMATCHED
                    } else {
                        depth -= 1;
                        BRACKETS[depth % 3]
                    }
                }
                _ => continue,
            };
            push(last, at, None, lead);
            lead = 0.0;
            push(at, at + 1, Some(color), 0.0);
            last = at + 1;
        }
        push(last, end, None, lead);
    }
    out
}

/// Highlighted, bracket-coloured layout for a code buffer. Memoised on the
/// text so it costs one hash per frame.
pub fn highlight_code(ui: &egui::Ui, font: f32, code: &str, syntax: &str) -> LayoutJob {
    use std::hash::{Hash, Hasher};
    static CACHE: Mutex<Vec<(u64, Arc<LayoutJob>)>> = Mutex::new(Vec::new());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut h);
    syntax.hash(&mut h);
    font.to_bits().hash(&mut h);
    let key = h.finish();
    if let Ok(cache) = CACHE.lock()
        && let Some((_, job)) = cache.iter().find(|(k, _)| *k == key) {
            return (**job).clone();
        }
    let job = if code.len() > 400_000 {
        LayoutJob::simple(code.to_owned(), egui::FontId::monospace(font), vs::TEXT, f32::INFINITY)
    } else {
        let theme = CodeTheme::dark(font);
        colorize_brackets(&highlight_with(ui.ctx(), ui.style(), &theme, code, syntax, syntect()))
    };
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(0, (key, Arc::new(job.clone())));
        cache.truncate(24);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_resolve() {
        for (file, want) in [
            ("lib/main.dart", "Dart"),
            ("index.php", "PHP"),
            ("app.js", "JavaScript"),
            ("style.css", "CSS"),
            ("index.html", "HTML"),
            ("main.ts", "TypeScript"),
            ("App.tsx", "TypeScript"),
            ("Cargo.toml", "TOML"),
            ("main.py", "Python"),
            ("query.sql", "SQL"),
            ("Main.kt", "Kotlin"),
            ("main.go", "Go"),
            ("README.md", "Markdown"),
            ("config.yml", "YAML"),
            ("Dockerfile", "Dockerfile"),
            ("theme.scss", "SCSS"),
        ] {
            let got = syntax_name(file);
            assert!(got.contains(want), "{file}: got {got}");
        }
    }

    #[test]
    fn brackets_get_depth_colours() {
        let string = Color32::from_rgb(0xce, 0x91, 0x78);
        let font = egui::FontId::monospace(12.0);
        let mut job = LayoutJob::default();
        job.append("f(a[0]) ", 0.0, egui::TextFormat::simple(font.clone(), vs::TEXT));
        job.append("\"(x)\"", 0.0, egui::TextFormat::simple(font, string));
        let out = colorize_brackets(&job);
        let color_at = |at: usize| {
            out.sections
                .iter()
                .find(|s| s.byte_range.start.0 <= at && at < s.byte_range.end.0)
                .unwrap()
                .format
                .color
        };
        assert_eq!(color_at(1), BRACKETS[0]); // (
        assert_eq!(color_at(3), BRACKETS[1]); // [
        assert_eq!(color_at(5), BRACKETS[1]); // ]
        assert_eq!(color_at(6), BRACKETS[0]); // )
        assert_eq!(color_at(9), string); // ( inside the string
    }
}
