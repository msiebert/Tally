//! Command line. With no arguments Tally serves MCP over stdio, which is how
//! agents launch it. `tally export` is for people.

use std::path::PathBuf;

pub const USAGE: &str = "\
tally: a list-making app for agents

USAGE:
    tally                    Serve MCP over stdin/stdout (how agents run it)
    tally export [OPTIONS]   Write all lists, items, tags and dependencies out
    tally --help | --version

EXPORT OPTIONS:
    --format <json|markdown>   Output format (default: json)
    --output <PATH>            Write to a new file instead of stdout
                               (refuses to overwrite an existing file)

The database is ~/.local/share/tally/tally.db, or the file named by TALLY_DB.
Backups are manual: export whenever you want a copy.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Markdown,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Export {
        format: Format,
        output: Option<PathBuf>,
    },
    Help,
    Version,
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Serve);
    };
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" => Ok(Command::Version),
        "export" => {
            let mut format = Format::Json;
            let mut output = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--format" | "-f" => {
                        let value = args
                            .next()
                            .ok_or("--format needs a value: json or markdown")?;
                        format = match value.as_str() {
                            "json" => Format::Json,
                            "markdown" | "md" => Format::Markdown,
                            other => {
                                return Err(format!(
                                    "unknown format '{other}': use json or markdown"
                                ));
                            }
                        };
                    }
                    "--output" | "-o" => {
                        let value = args.next().ok_or("--output needs a file path")?;
                        output = Some(PathBuf::from(value));
                    }
                    other => return Err(format!("unknown option '{other}' for export")),
                }
            }
            Ok(Command::Export { format, output })
        }
        other => Err(format!("unknown command '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_serves() {
        assert_eq!(parse_all(&[]), Ok(Command::Serve));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_all(&["--help"]), Ok(Command::Help));
        assert_eq!(parse_all(&["-h"]), Ok(Command::Help));
        assert_eq!(parse_all(&["--version"]), Ok(Command::Version));
    }

    #[test]
    fn export_defaults_to_json_on_stdout() {
        assert_eq!(
            parse_all(&["export"]),
            Ok(Command::Export {
                format: Format::Json,
                output: None
            })
        );
    }

    #[test]
    fn export_options() {
        assert_eq!(
            parse_all(&["export", "--format", "markdown", "--output", "out.md"]),
            Ok(Command::Export {
                format: Format::Markdown,
                output: Some(PathBuf::from("out.md"))
            })
        );
        assert_eq!(
            parse_all(&["export", "-f", "md"]),
            Ok(Command::Export {
                format: Format::Markdown,
                output: None
            })
        );
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(parse_all(&["frobnicate"]).is_err());
        assert!(parse_all(&["export", "--format"]).is_err());
        assert!(parse_all(&["export", "--format", "xml"]).is_err());
        assert!(parse_all(&["export", "--output"]).is_err());
        assert!(parse_all(&["export", "--wat"]).is_err());
    }
}
