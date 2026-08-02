use std::io::{self, IsTerminal, Read, Write};
use tokcost::render::{self, Json, JsonObject};
use tokcost::{meter, pricing, vocab};

const DEFAULT_MODEL: &str = "gpt-4o";

#[derive(Debug, PartialEq)]
enum Command {
    Count {
        model: String,
        json: bool,
        paths: Vec<String>,
    },
    Meter {
        model: String,
        command: String,
        args: Vec<String>,
    },
    Help,
    Version,
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match parse(&argv) {
        Ok(Command::Help) => print_help(),
        Ok(Command::Version) => println!("tokcost {}", env!("CARGO_PKG_VERSION")),
        Ok(Command::Count { model, json, paths }) => {
            let ok = run_count(&model, json, &paths);
            std::process::exit(if ok { 0 } else { 1 });
        }
        Ok(Command::Meter {
            model,
            command,
            args,
        }) => {
            let options = meter::MeterOptions {
                model: &model,
                command: &command,
                args: &args,
            };
            match meter::run(&options) {
                Ok(code) => std::process::exit(code),
                Err(e) => {
                    eprintln!("tokcost: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(msg) => {
            eprintln!("tokcost: {msg}");
            eprintln!("Try 'tokcost --help' for usage.");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------

fn parse(argv: &[String]) -> Result<Command, String> {
    if argv.first().map(String::as_str) == Some("meter") {
        parse_meter(&argv[1..])
    } else {
        parse_count(argv)
    }
}

fn parse_count(argv: &[String]) -> Result<Command, String> {
    let mut model = DEFAULT_MODEL.to_string();
    let mut json = false;
    let mut paths = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--json" => {
                json = true;
                i += 1;
            }
            "--model" | "-m" => {
                i += 1;
                model = argv
                    .get(i)
                    .ok_or_else(|| "--model requires a value".to_string())?
                    .clone();
                i += 1;
            }
            arg if arg.starts_with("--model=") => {
                model = arg["--model=".len()..].to_string();
                i += 1;
            }
            "--" => {
                paths.extend(argv[i + 1..].iter().cloned());
                i = argv.len();
            }
            arg if arg.starts_with('-') && arg.len() > 1 => {
                return Err(format!("unknown flag: {arg}"));
            }
            other => {
                paths.push(other.to_string());
                i += 1;
            }
        }
    }
    Ok(Command::Count { model, json, paths })
}

fn parse_meter(argv: &[String]) -> Result<Command, String> {
    let mut model = DEFAULT_MODEL.to_string();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--" => {
                let rest = &argv[i + 1..];
                let (command, cmd_args) = rest
                    .split_first()
                    .ok_or_else(|| "meter requires a command after '--'".to_string())?;
                return Ok(Command::Meter {
                    model,
                    command: command.clone(),
                    args: cmd_args.to_vec(),
                });
            }
            "-h" | "--help" => return Ok(Command::Help),
            "--model" | "-m" => {
                i += 1;
                model = argv
                    .get(i)
                    .ok_or_else(|| "--model requires a value".to_string())?
                    .clone();
                i += 1;
            }
            arg if arg.starts_with("--model=") => {
                model = arg["--model=".len()..].to_string();
                i += 1;
            }
            other => return Err(format!("unexpected argument before '--': {other}")),
        }
    }
    Err("meter requires '--' followed by a command".to_string())
}

// ---------------------------------------------------------------------
// Counting
// ---------------------------------------------------------------------

struct FileResult {
    path: String,
    count: vocab::Count,
    cost: Option<pricing::Cost>,
}

/// Read, count, and print results for `paths` (or stdin if empty). Returns
/// `false` if any input couldn't be read, so `main` can pick a non-zero
/// exit code while still reporting every input it could read.
fn run_count(model: &str, json: bool, paths: &[String]) -> bool {
    let inputs: Vec<String> = if paths.is_empty() {
        vec!["-".to_string()]
    } else {
        paths.to_vec()
    };

    let mut results = Vec::new();
    let mut had_error = false;

    for path in &inputs {
        match read_input(path) {
            Ok(text) => {
                let count = vocab::count(&text, model);
                let cost = pricing::estimate_cost(model, count.value(), 0);
                results.push(FileResult {
                    path: path.clone(),
                    count,
                    cost,
                });
            }
            Err(e) => {
                eprintln!("tokcost: {path}: {e}");
                had_error = true;
            }
        }
    }

    if json {
        print_json(model, &results);
    } else {
        print_human(model, &results);
    }

    !had_error
}

fn read_input(path: &str) -> io::Result<String> {
    let bytes = if path == "-" {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf)?;
        buf
    } else {
        std::fs::read(path)?
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn print_human(model: &str, results: &[FileResult]) {
    // Only colorize when stdout is a real terminal, so piping output to a
    // file or another command never leaks raw ANSI escapes into it.
    let color = io::stdout().is_terminal();
    let paint = |text: &str, code: &str| -> String {
        if color {
            render::colorize(text, code)
        } else {
            text.to_string()
        }
    };

    let mut total_tokens = 0usize;
    let mut total_cost = 0.0f64;
    let mut any_estimated = false;
    let mut any_priced = false;

    for r in results {
        any_estimated |= r.count.is_estimated();
        total_tokens += r.count.value();
        let label = if r.count.is_estimated() {
            "~tokens"
        } else {
            "tokens"
        };
        let cost_str = match r.cost {
            Some(c) => {
                any_priced = true;
                total_cost += c.total_usd();
                render::format_usd(c.total_usd())
            }
            None => "n/a".to_string(),
        };
        println!(
            "{}: {} {label} {} {cost_str}",
            paint(&r.path, render::BOLD),
            render::format_int(r.count.value() as u64),
            paint("\u{b7}", render::DIM),
        );
    }

    if results.len() > 1 {
        let total_label = if any_estimated { "~tokens" } else { "tokens" };
        let total_cost_str = if any_priced {
            render::format_usd(total_cost)
        } else {
            "n/a".to_string()
        };
        println!(
            "{}: {} {total_label} {} {total_cost_str}",
            paint("total", render::BOLD),
            render::format_int(total_tokens as u64),
            paint("\u{b7}", render::DIM),
        );
    }
    if any_estimated {
        let note =
            format!("note: {model} token counts are a calibrated estimate, not exact (see README)");
        eprintln!("{}", paint(&note, render::YELLOW));
    }
}

fn print_json(model: &str, results: &[FileResult]) {
    let mut total_tokens = 0usize;
    let mut total_cost = 0.0f64;
    let mut any_priced = false;

    let files: Vec<Json> = results
        .iter()
        .map(|r| {
            total_tokens += r.count.value();
            let cost_field = match r.cost {
                Some(c) => {
                    any_priced = true;
                    total_cost += c.total_usd();
                    Json::Num(render::round_money(c.total_usd()))
                }
                None => Json::Null,
            };
            JsonObject::new()
                .field("path", r.path.as_str())
                .field("tokens", r.count.value())
                .field("exact", !r.count.is_estimated())
                .field("cost_usd", cost_field)
                .build()
        })
        .collect();

    let root = JsonObject::new()
        .field("model", model)
        .field("pricing_as_of", pricing::PRICING_AS_OF)
        .field("files", Json::Array(files))
        .field("total_tokens", total_tokens)
        .field(
            "total_cost_usd",
            if any_priced {
                Json::Num(render::round_money(total_cost))
            } else {
                Json::Null
            },
        )
        .build();

    println!("{}", root.to_json_string());
}

fn print_help() {
    // Real embedded newlines (not `\`-continuations, which strip all
    // leading whitespace off the following line and would eat this
    // indentation) so the layout below is exactly what gets printed.
    print!(
        "tokcost {version} — exact token counts and cost estimates for LLM text

USAGE:
    tokcost [OPTIONS] [FILE...]
    tokcost meter [OPTIONS] -- COMMAND [ARGS...]
    command | tokcost [OPTIONS]

OPTIONS:
    -m, --model <MODEL>   Model to count/price for (default: {default_model})
        --json            Emit machine-readable JSON instead of text
    -h, --help            Show this help
    -V, --version         Show version

EXAMPLES:
    tokcost src/main.rs
    cat prompt.txt | tokcost --model claude-sonnet-5
    tokcost meter --model gpt-4o -- python chat.py

Set TK_PRICES to override built-in pricing, e.g.:
    TK_PRICES=\"gpt-4o=2.50:10.00\" tokcost --model gpt-4o file.txt
",
        version = env!("CARGO_PKG_VERSION"),
        default_model = DEFAULT_MODEL,
    );
    io::stdout().flush().ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_count_defaults_with_no_args() {
        assert_eq!(
            parse(&[]).unwrap(),
            Command::Count {
                model: DEFAULT_MODEL.to_string(),
                json: false,
                paths: vec![],
            }
        );
    }

    #[test]
    fn parse_count_collects_paths_and_flags() {
        let argv = s(&["--model", "claude-sonnet-5", "--json", "a.txt", "b.txt"]);
        assert_eq!(
            parse(&argv).unwrap(),
            Command::Count {
                model: "claude-sonnet-5".to_string(),
                json: true,
                paths: vec!["a.txt".to_string(), "b.txt".to_string()],
            }
        );
    }

    #[test]
    fn parse_count_supports_model_equals_syntax() {
        let argv = s(&["--model=gpt-4o", "file.txt"]);
        assert_eq!(
            parse(&argv).unwrap(),
            Command::Count {
                model: "gpt-4o".to_string(),
                json: false,
                paths: vec!["file.txt".to_string()],
            }
        );
    }

    #[test]
    fn parse_count_double_dash_treats_rest_as_paths() {
        let argv = s(&["--", "--weird-filename", "-m"]);
        assert_eq!(
            parse(&argv).unwrap(),
            Command::Count {
                model: DEFAULT_MODEL.to_string(),
                json: false,
                paths: vec!["--weird-filename".to_string(), "-m".to_string()],
            }
        );
    }

    #[test]
    fn parse_count_lone_dash_is_a_stdin_path() {
        let argv = s(&["-"]);
        assert_eq!(
            parse(&argv).unwrap(),
            Command::Count {
                model: DEFAULT_MODEL.to_string(),
                json: false,
                paths: vec!["-".to_string()],
            }
        );
    }

    #[test]
    fn parse_count_rejects_unknown_flags() {
        assert!(parse(&s(&["--bogus"])).is_err());
    }

    #[test]
    fn parse_count_requires_model_value() {
        assert!(parse(&s(&["--model"])).is_err());
    }

    #[test]
    fn parse_help_and_version() {
        assert_eq!(parse(&s(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse(&s(&["-V"])).unwrap(), Command::Version);
    }

    #[test]
    fn parse_meter_requires_double_dash_and_command() {
        assert!(parse(&s(&["meter"])).is_err());
        assert!(parse(&s(&["meter", "--"])).is_err());
    }

    #[test]
    fn parse_meter_collects_model_command_and_args() {
        let argv = s(&[
            "meter", "--model", "gpt-4o", "--", "python", "chat.py", "-v",
        ]);
        assert_eq!(
            parse(&argv).unwrap(),
            Command::Meter {
                model: "gpt-4o".to_string(),
                command: "python".to_string(),
                args: vec!["chat.py".to_string(), "-v".to_string()],
            }
        );
    }

    #[test]
    fn parse_meter_rejects_flags_after_double_dash_as_meters_own() {
        // Everything after '--' belongs to the wrapped command, including
        // things that look like tokcost's own flags.
        let argv = s(&["meter", "--", "echo", "--model"]);
        assert_eq!(
            parse(&argv).unwrap(),
            Command::Meter {
                model: DEFAULT_MODEL.to_string(),
                command: "echo".to_string(),
                args: vec!["--model".to_string()],
            }
        );
    }

    #[test]
    fn read_input_reads_a_real_file() {
        let path = std::env::temp_dir().join(format!("tokcost-test-{}.txt", std::process::id()));
        std::fs::write(&path, "hello from a file\n").unwrap();
        let text = read_input(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(text, "hello from a file\n");
    }

    #[test]
    fn read_input_reports_missing_file_as_error() {
        assert!(read_input("/nonexistent/tokcost-test-path").is_err());
    }
}
