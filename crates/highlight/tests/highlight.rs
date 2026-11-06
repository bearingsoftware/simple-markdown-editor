//! Fenced-code highlighting (MDEDIT-190).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use mdedit_highlight::highlight;
use mdedit_language::Token;
use mdedit_offsets::ByteSpan;
use proptest::prelude::*;

fn tokens_of(info: &str, code: &str) -> Vec<(String, Token)> {
    highlight(info, code)
        .unwrap_or_else(|| panic!("{info:?} should be a known language"))
        .into_iter()
        .map(|(span, token)| (span.slice(code).to_string(), token))
        .collect()
}

fn has(found: &[(String, Token)], text: &str, token: Token) -> bool {
    found.iter().any(|(t, k)| *k == token && t.contains(text))
}

/// The invariants every result owes the caller, whatever the input was.
fn assert_invariants(code: &str, spans: &[(ByteSpan, Token)]) {
    let mut previous_end = 0usize;
    for (span, _) in spans {
        let (start, end) = (span.start().get(), span.end().get());
        assert!(start < end, "empty span {start}..{end}");
        assert!(start >= previous_end, "overlap or disorder at {start}");
        assert!(end <= code.len(), "span {start}..{end} past {}", code.len());
        assert!(code.is_char_boundary(start), "{start} splits a character");
        assert!(code.is_char_boundary(end), "{end} splits a character");
        previous_end = end;
    }
}

#[test]
fn rust_has_a_keyword_a_string_and_a_comment() {
    let found = tokens_of("rust", "// note\nfn main() {\n    let s = \"hi\";\n}\n");
    assert!(has(&found, "// note", Token::Comment), "{found:?}");
    assert!(has(&found, "fn", Token::Key), "{found:?}");
    assert!(has(&found, "\"hi\"", Token::String), "{found:?}");
}

#[test]
fn json_has_a_string_and_a_number() {
    let found = tokens_of("json", "{\"a\": [1, true, \"x\"]}\n");
    assert!(has(&found, "\"x\"", Token::String), "{found:?}");
    assert!(has(&found, "1", Token::Number), "{found:?}");
}

#[test]
fn python_has_a_keyword_a_string_and_a_comment() {
    let found = tokens_of("python", "# note\ndef f():\n    return 'x'\n");
    assert!(has(&found, "# note", Token::Comment), "{found:?}");
    assert!(has(&found, "def", Token::Key), "{found:?}");
    assert!(has(&found, "'x'", Token::String), "{found:?}");
}

#[test]
fn shell_has_a_keyword_a_string_and_a_comment() {
    let found = tokens_of("sh", "# note\nif true; then echo \"x\"; fi\n");
    assert!(has(&found, "# note", Token::Comment), "{found:?}");
    assert!(has(&found, "if", Token::Key), "{found:?}");
    assert!(has(&found, "\"x\"", Token::String), "{found:?}");
}

#[test]
fn the_info_string_is_case_insensitive_and_only_its_first_word_counts() {
    let found = tokens_of("  Rust   no_run ", "fn f() {}\n");
    assert!(has(&found, "fn", Token::Key), "{found:?}");
}

#[test]
fn an_unknown_info_string_is_none() {
    assert!(highlight("not-a-language-at-all", "fn main() {}\n").is_none());
    assert!(highlight(&"x".repeat(10_000), "fn main() {}\n").is_none());
}

#[test]
fn language_is_never_sniffed() {
    let json = "{\"a\": 1, \"b\": [true, null]}\n";
    assert!(highlight("", json).is_none());
    assert!(highlight("   ", json).is_none());

    for info in ["text", "txt"] {
        let spans = highlight(info, json).unwrap_or_default();
        assert!(
            spans.iter().all(|(_, token)| *token != Token::Number),
            "a JSON body under `{info}` must not be highlighted as JSON: {spans:?}"
        );
    }
}

#[test]
fn mid_edit_fences_still_satisfy_the_invariants() {
    let cases: &[(&str, &str)] = &[
        ("rust", "let s = \"never closed\n"),
        ("rust", "/* never closed\nfn main() {\n"),
        ("rust", "fn mai"),
        ("rust", "let x = 1;\nfn f() {\n    if x"),
        ("json", "{\"a\": \"unterminated"),
        ("json", "{\"a\": [1, 2,"),
        ("python", "def f(:\n    \"\"\"never closed\n"),
        ("sh", "echo \"never closed\nif ["),
        ("yaml", "a: [1, 2\nb: \"x\n  - c"),
        ("html", "<div class=\"a"),
        ("c", "#include <std"),
        ("js", "const x = `template ${"),
    ];
    for (info, code) in cases {
        let spans = highlight(info, code).unwrap_or_default();
        assert_invariants(code, &spans);
    }
}

#[test]
fn awkward_text_satisfies_the_invariants() {
    let long = "a".repeat(10_000);
    let long_string = format!("let s = \"{long}\";\n");
    let cases: Vec<String> = vec![
        "fn f() {}\r\nlet s = \"x\";\r\n// c\r\n".into(),
        "fn f() {}\r\n".into(),
        "\r\n\r\n".into(),
        "let s = \"héllo wörld 日本語 🦀\"; // 🦀 comment\n".into(),
        "// 日本語\nfn 日本() {}\n".into(),
        "\u{feff}fn main() {}\n".into(),
        long.clone(),
        long_string,
        format!("// {long}\nfn f() {{}}\n"),
        "\n".into(),
        String::new(),
        "\0\0\0\n".into(),
        "fn f() {}".into(),
        "\"".into(),
        "'".into(),
        "/*".into(),
        "r#\"".into(),
        "((((((((((((((((((((((((((((((((((((((((((((((((((\n".into(),
        "\\\n\\\n\\\n".into(),
        "    \t  \n".into(),
    ];
    for info in ["rust", "json", "python", "sh", "js", "html", "yaml"] {
        for code in &cases {
            let spans = highlight(info, code).unwrap_or_default();
            assert_invariants(code, &spans);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_text_in_any_language_satisfies_the_invariants(
        code in "(?s).{0,200}",
        info in prop::sample::select(vec!["rust", "json", "python", "sh", "js", "html", "toml"]),
    ) {
        let spans = highlight(info, &code).unwrap_or_default();
        assert_invariants(&code, &spans);
    }

    #[test]
    fn code_shaped_text_satisfies_the_invariants(
        code in r#"([a-z{}()\[\]"'#/*\\ \t\r\n日é🦀]|fn |let |if |<|>|=){0,120}"#,
        info in prop::sample::select(vec!["rust", "json", "python", "sh", "js", "html"]),
    ) {
        let spans = highlight(info, &code).unwrap_or_default();
        assert_invariants(&code, &spans);
    }
}

/// Parses a few sample lines with every bundled syntax and prints the ones that error, for the
/// decision record. Run it with `--run-ignored only --no-capture`.
#[test]
#[ignore = "prints a list for the decision record"]
fn every_bundled_syntax_parses() {
    let samples = [
        "// comment",
        "# comment",
        "let x = \"string\" + 12;",
        "fn main() { if (a) { return; } }",
        "<div class=\"a\">text</div>",
        "{\"a\": [1, true, null]}",
        "def f(x): return x * 2",
        "SELECT * FROM t WHERE a = 'b';",
        "/* block */ int main(void) { return 0; }",
        "`${x}` \"\"\"doc\"\"\" r#\"raw\"# <<EOF",
    ];
    let failing = mdedit_highlight::syntaxes_that_error(&samples);
    println!("{} syntaxes error under fancy-regex:", failing.len());
    for name in &failing {
        println!("  {name}");
    }
}

/// Prints every language the bundled set covers, for the decision record. Run it with
/// `--run-ignored only --no-capture`.
#[test]
#[ignore = "prints a list for the decision record"]
fn bundled_languages() {
    let names = mdedit_highlight::language_names();
    println!("{} languages:", names.len());
    println!("{}", names.join(", "));
    for wanted in [
        "toml",
        "ts",
        "typescript",
        "dockerfile",
        "powershell",
        "ps1",
        "bash",
        "sh",
        "json",
        "yaml",
        "rust",
        "python",
    ] {
        println!("{wanted}: {}", highlight(wanted, "x\n").is_some());
    }
}

/// Times `highlight` alone, in a release build, to choose the size cap.
#[test]
#[ignore = "a measurement"]
fn highlight_cost() {
    let line = "    let value_1: u32 = compute(\"name 1\", 1); // note 1\n";
    let _ = highlight("rust", "fn main() {}\n");
    for lines in [10, 30, 100, 300, 1000, 3000] {
        let code = line.repeat(lines);
        let started = std::time::Instant::now();
        let spans = highlight("rust", &code).unwrap();
        println!(
            "{lines:>5} lines ({:>6} bytes): {:>8.2} ms, {} spans",
            code.len(),
            started.elapsed().as_secs_f64() * 1000.0,
            spans.len()
        );
    }
    let long = format!("let s = \"{}\";\n", "a".repeat(3000));
    let started = std::time::Instant::now();
    let _ = highlight("rust", &long);
    println!(
        "one 3000 char line: {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// Times the first `highlight` call, which loads the syntax set. One load per process, so run the
/// test binary repeatedly and take the median.
#[test]
#[ignore = "a measurement"]
fn cold_load() {
    let started = std::time::Instant::now();
    let _ = highlight("rust", "fn main() {}\n");
    println!(
        "cold load: {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

const OLD_SAMPLES: [&str; 10] = [
    "// comment",
    "# comment",
    "let x = \"string\" + 12;",
    "fn main() { if (a) { return; } }",
    "<div class=\"a\">text</div>",
    "{\"a\": [1, true, null]}",
    "def f(x): return x * 2",
    "SELECT * FROM t WHERE a = 'b';",
    "/* block */ int main(void) { return 0; }",
    "`${x}` \"\"\"doc\"\"\" r#\"raw\"# <<EOF",
];

const VENDORED: [&str; 4] = ["TOML", "Dockerfile", "Kotlin", "PowerShell"];

#[test]
fn toml_has_a_key_a_string_and_a_comment() {
    let found = tokens_of("toml", "# note\n[package]\nname = \"x\"\nn = 1\n");
    assert!(has(&found, "# note", Token::Comment), "{found:?}");
    assert!(has(&found, "\"x\"", Token::String), "{found:?}");
    assert!(has(&found, "name", Token::Tag), "{found:?}");
    assert!(has(&found, "1", Token::Number), "{found:?}");
}

#[test]
fn dockerfile_has_an_instruction_a_string_and_a_comment() {
    let found = tokens_of("dockerfile", "# note\nFROM rust:1\nLABEL a=\"b\"\n");
    assert!(has(&found, "# note", Token::Comment), "{found:?}");
    assert!(has(&found, "FROM", Token::Key), "{found:?}");
    assert!(has(&found, "\"b\"", Token::String), "{found:?}");
}

#[test]
fn kotlin_has_a_keyword_a_string_and_a_comment() {
    let found = tokens_of("kotlin", "// note\nfun main() {\n    val s = \"hi\"\n}\n");
    assert!(has(&found, "// note", Token::Comment), "{found:?}");
    assert!(has(&found, "fun", Token::Key), "{found:?}");
    assert!(has(&found, "\"hi\"", Token::String), "{found:?}");
}

#[test]
fn powershell_has_a_keyword_a_string_and_a_comment() {
    let found = tokens_of(
        "powershell",
        "# note\nfunction Get-Thing {\n    $s = \"hi\"\n}\n",
    );
    assert!(has(&found, "# note", Token::Comment), "{found:?}");
    assert!(has(&found, "function", Token::Key), "{found:?}");
    assert!(has(&found, "\"hi\"", Token::String), "{found:?}");
}

#[test]
fn aliases_name_a_language_and_colour_a_keyword() {
    for (info, code, keyword) in [
        ("ts", "const x = 1;\n", "const"),
        ("tsx", "const x = 1;\n", "const"),
        ("typescript", "const x = 1;\n", "const"),
        ("jsx", "const x = 1;\n", "const"),
        ("mts", "const x = 1;\n", "const"),
        ("cts", "const x = 1;\n", "const"),
        ("pwsh", "function F {}\n", "function"),
        ("ps1", "function F {}\n", "function"),
        ("ps", "function F {}\n", "function"),
        ("kt", "fun f() {}\n", "fun"),
        ("kts", "fun f() {}\n", "fun"),
        ("docker", "FROM a\n", "FROM"),
    ] {
        let found = tokens_of(info, code);
        assert!(has(&found, keyword, Token::Key), "{info}: {found:?}");
    }
}

#[test]
fn typescript_is_drawn_exactly_as_javascript() {
    let code = "const f = (a, b) => `t${a}`; // note\nlet s = \"x\";\n";
    assert_eq!(highlight("ts", code), highlight("js", code));
    assert!(highlight("js", code).is_some_and(|spans| !spans.is_empty()));
}

#[test]
fn vendored_syntaxes_parse_their_samples() {
    let own: [(&str, &[&str]); 4] = [
        (
            "TOML",
            &[
                "[server.tls]\nkey = \"v\" # c\nports = [1, 2]\nt = { a = 1.5, b = true }",
                "dt = 1979-05-27T07:32:00Z\nmulti = '''\nline'''",
            ],
        ),
        (
            "Dockerfile",
            &[
                "FROM rust:1 AS build\nRUN cargo build --release && echo \"done\"",
                "ENV A=b \\",
                "    C=d",
                "COPY --from=build /a /b\nCMD [\"x\", \"y\"]",
            ],
        ),
        (
            "Kotlin",
            &[
                "package a.b\nclass C<T>(val x: Int) : Base() { fun f(): String = \"$x ${x + 1}\" }",
                "val r = \"\"\"raw\"\"\".trimIndent() // c\n@Suppress(\"x\") suspend fun g() = when (x) { is A -> 1; else -> 2 }",
            ],
        ),
        (
            "PowerShell",
            &[
                "function Get-Thing { param([string]$Name = 'x') Write-Host \"hi $Name\" }",
                "$a = @{ k = 1; l = 0x1F }; if ($a -eq 2) { 1..3 | ForEach-Object { $_ * 2 } } # c\n<# block #>",
            ],
        ),
    ];
    let mut broken = Vec::new();
    for (name, lines) in own {
        let mut samples: Vec<&str> = OLD_SAMPLES.to_vec();
        samples.extend(lines);
        if !mdedit_highlight::syntaxes_that_error_in(&[name], &samples).is_empty() {
            broken.push(name);
        }
    }
    assert!(broken.is_empty(), "error under fancy-regex: {broken:?}");
    let names = mdedit_highlight::language_names();
    for name in VENDORED {
        assert!(names.iter().any(|n| n == name), "{name} is not bundled");
    }
}

#[test]
fn vendored_syntaxes_use_no_features_syntect_5_ignores() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("syntaxes");
    let mut checked = 0;
    for dir in std::fs::read_dir(&root).unwrap() {
        for file in std::fs::read_dir(dir.unwrap().path()).unwrap() {
            let path = file.unwrap().path();
            if path.extension().is_none_or(|e| e != "sublime-syntax") {
                continue;
            }
            checked += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let key = line.trim_start().trim_start_matches("- ").trim_start();
                let bad = key.starts_with("extends:")
                    || key.starts_with("branch_point:")
                    || key.starts_with("fail:")
                    || key.strip_prefix("pop:").is_some_and(|v| {
                        v.trim().chars().next().is_some_and(|c| c.is_ascii_digit())
                    });
                assert!(!bad, "{}:{}: {line}", path.display(), n + 1);
            }
        }
    }
    assert_eq!(checked, VENDORED.len());
}

/// Times each (syntax, sample line) pair, first call per syntax included, to find where
/// `vendored_syntaxes_parse_their_samples` spends its time.
#[test]
#[ignore = "a measurement"]
fn sample_timings() {
    let started = std::time::Instant::now();
    let _ = highlight("rust", "x\n");
    println!("load: {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    let lines = [
        "// comment",
        "let x = \"string\" + 12;",
        "<div class=\"a\">text</div>",
        "`${x}` \"\"\"doc\"\"\" r#\"raw\"# <<EOF",
        "[server.tls]\nkey = \"v\" # c\nports = [1, 2]",
        "FROM rust:1 AS build\nRUN cargo build --release && echo \"done\"",
        "package a.b\nclass C<T>(val x: Int) : Base() { fun f(): String = \"$x ${x + 1}\" }",
        "$a = @{ k = 1; l = 0x1F }; if ($a -eq 2) { 1..3 | ForEach-Object { $_ * 2 } } # c\n<# block #>",
    ];
    for info in ["toml", "dockerfile", "kotlin", "powershell"] {
        for (i, line) in lines.iter().chain(lines.iter()).enumerate() {
            let started = std::time::Instant::now();
            let _ = highlight(info, line);
            println!(
                "{info:>10} sample {i}: {:>8.2} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
