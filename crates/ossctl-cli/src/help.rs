//! Machine-readable CLI help derived from clap's command tree.
//!
//! Text help remains entirely clap-owned. This module is entered only when a
//! successful clap help display also contains the global `--json` flag.

use std::ffi::OsString;

use clap::{Arg, Command, CommandFactory};
use serde::Serialize;

use crate::cli::Cli;
use crate::error::CliError;

const HELP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
struct HelpDocument {
    schema_version: u32,
    command: CommandHelp,
}

#[derive(Debug, Serialize)]
struct CommandHelp {
    name: String,
    path: Vec<String>,
    description: Option<String>,
    usage: String,
    subcommands: Vec<SubcommandHelp>,
    flags: Vec<FlagHelp>,
    args: Vec<ArgumentHelp>,
    examples: Vec<Example>,
    exit_codes: Vec<ExitCodeHelp>,
    deprecated: bool,
}

#[derive(Debug, Serialize)]
struct SubcommandHelp {
    name: String,
    description: Option<String>,
    aliases: Vec<String>,
    deprecated: bool,
}

#[derive(Debug, Serialize)]
#[allow(clippy::struct_excessive_bools)] // These independent booleans are the public help schema.
struct FlagHelp {
    id: String,
    long: Option<String>,
    short: Option<char>,
    description: Option<String>,
    required: bool,
    global: bool,
    hidden: bool,
    takes_value: bool,
    value_names: Vec<String>,
    defaults: Vec<String>,
    env: Option<String>,
    accepted_values: Vec<String>,
    deprecated: bool,
}

#[derive(Debug, Serialize)]
struct ArgumentHelp {
    id: String,
    index: usize,
    description: Option<String>,
    required: bool,
    hidden: bool,
    value_names: Vec<String>,
    defaults: Vec<String>,
    env: Option<String>,
    accepted_values: Vec<String>,
    deprecated: bool,
}

#[derive(Debug, Serialize)]
struct Example {
    description: &'static str,
    argv: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ExitCodeHelp {
    code: u8,
    meaning: &'static str,
}

/// Emit structured help for the command selected by a clap help invocation.
pub(crate) fn emit(argv: &[OsString]) -> Result<(), CliError> {
    let mut root = Cli::command();
    root.build();
    let (command, path) = selected_command(&root, argv);
    let document = HelpDocument {
        schema_version: HELP_SCHEMA_VERSION,
        command: command_help(command, &path)?,
    };
    crate::output::emit_json(&document, &[])
}

/// Follow clap-resolved subcommand spellings until help terminates parsing.
/// Intermediate ossctl commands contain no value-taking options (guarded by a
/// unit test), so non-option tokens can only be subcommands at those levels.
fn selected_command<'a>(root: &'a Command, argv: &[OsString]) -> (&'a Command, Vec<String>) {
    let mut command = root;
    let mut path = vec![root.get_name().to_string()];

    for token in argv.iter().skip(1) {
        let Some(token) = token.to_str() else {
            break;
        };
        if matches!(token, "--" | "--help" | "-h") {
            break;
        }
        if let Some(subcommand) = command.get_subcommands().find(|candidate| {
            candidate.get_name() == token || candidate.get_all_aliases().any(|alias| alias == token)
        }) {
            command = subcommand;
            // Extras are keyed by the canonical path, never by an alias spelling.
            path.push(command.get_name().to_string());
        } else if !token.starts_with('-') {
            break;
        }
    }

    (command, path)
}

fn command_help(command: &Command, path: &[String]) -> Result<CommandHelp, CliError> {
    let mut rendered = command.clone();
    let flags = command
        .get_arguments()
        .filter(|arg| !arg.is_positional())
        .map(|arg| flag_help(arg, path))
        .collect();
    let args = command
        .get_arguments()
        .filter(|arg| arg.is_positional())
        .map(argument_help)
        .collect();

    Ok(CommandHelp {
        name: command.get_name().to_string(),
        path: path.to_vec(),
        description: text(command.get_long_about().or_else(|| command.get_about())),
        usage: rendered.render_usage().to_string(),
        subcommands: command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
            .map(|subcommand| SubcommandHelp {
                name: subcommand.get_name().to_string(),
                description: text(subcommand.get_about()),
                aliases: subcommand
                    .get_visible_aliases()
                    .map(str::to_string)
                    .collect(),
                deprecated: false,
            })
            .collect(),
        flags,
        args,
        examples: examples(path).ok_or_else(|| {
            CliError::system(
                "internal_help_examples",
                format!(
                    "no structured-help example registered for `{}`",
                    path.join(" ")
                ),
            )
        })?,
        exit_codes: vec![
            ExitCodeHelp {
                code: 0,
                meaning: "success, including help and version display",
            },
            ExitCodeHelp {
                code: 1,
                meaning: "caller- or domain-actionable error",
            },
            ExitCodeHelp {
                code: 2,
                meaning: "system or internal error",
            },
        ],
        // The current public CLI has no deprecated commands or arguments.
        // Add an extras registry before introducing the first deprecation.
        deprecated: false,
    })
}

fn flag_help(arg: &Arg, path: &[String]) -> FlagHelp {
    let takes_value = arg.get_action().takes_values();
    let mut accepted_values = accepted_values(arg);
    // `--bump` predates clap-level enum parsing, so changing its parser would
    // alter text help and error behavior. Surface the core protocol's finite
    // set here without changing that established CLI contract.
    if accepted_values.is_empty()
        && arg.get_id() == "bump"
        && matches!(path, [root, command, action] if root == "ossctl" && command == "release" && matches!(action.as_str(), "plan" | "cut"))
    {
        accepted_values = ossctl_core::protocol::plan::BumpLevel::VALID
            .iter()
            .map(ToString::to_string)
            .collect();
    }

    FlagHelp {
        id: arg.get_id().to_string(),
        long: arg.get_long().map(str::to_string),
        short: arg.get_short(),
        description: text(arg.get_long_help().or_else(|| arg.get_help())),
        required: arg.is_required_set(),
        global: arg.is_global_set(),
        hidden: arg.is_hide_set(),
        takes_value,
        value_names: if takes_value {
            value_names(arg)
        } else {
            Vec::new()
        },
        defaults: defaults(arg),
        env: arg
            .get_env()
            .map(|value| value.to_string_lossy().into_owned()),
        accepted_values,
        deprecated: false,
    }
}

fn argument_help(arg: &Arg) -> ArgumentHelp {
    ArgumentHelp {
        id: arg.get_id().to_string(),
        index: arg
            .get_index()
            .expect("clap positional arguments have an index"),
        description: text(arg.get_long_help().or_else(|| arg.get_help())),
        required: arg.is_required_set(),
        hidden: arg.is_hide_set(),
        value_names: value_names(arg),
        defaults: defaults(arg),
        env: arg
            .get_env()
            .map(|value| value.to_string_lossy().into_owned()),
        accepted_values: accepted_values(arg),
        deprecated: false,
    }
}

fn text(value: Option<&clap::builder::StyledStr>) -> Option<String> {
    value.map(ToString::to_string)
}

fn value_names(arg: &Arg) -> Vec<String> {
    arg.get_value_names()
        .into_iter()
        .flatten()
        .map(ToString::to_string)
        .collect()
}

fn defaults(arg: &Arg) -> Vec<String> {
    arg.get_default_values()
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect()
}

fn accepted_values(arg: &Arg) -> Vec<String> {
    arg.get_possible_values()
        .into_iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| value.get_name().to_string())
        .collect()
}

/// Examples are the only help metadata clap does not model independently of
/// rendered prose. Keeping this exhaustive match means adding a command cannot
/// silently ship without the canon-required structured example.
#[allow(clippy::too_many_lines)] // Exhaustive command-to-example registry is clearer as one table.
fn examples(path: &[String]) -> Option<Vec<Example>> {
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    let (description, argv): (&'static str, &'static [&'static str]) = match path.as_slice() {
        ["ossctl"] => (
            "Inspect the installed CLI version",
            &["ossctl", "version", "--json"],
        ),
        ["ossctl", "config"] => (
            "Inspect resolved configuration",
            &["ossctl", "config", "show", "--json"],
        ),
        ["ossctl", "config", "path"] => (
            "Print resolved project paths",
            &["ossctl", "config", "path"],
        ),
        ["ossctl", "config", "show"] => (
            "Inspect resolved paths and provenance",
            &["ossctl", "config", "show", "--json"],
        ),
        ["ossctl", "contract"] => (
            "Normalize the release contract",
            &["ossctl", "contract", "show", "--json"],
        ),
        ["ossctl", "contract", "show"] => (
            "Normalize the current repository contract",
            &["ossctl", "contract", "show", "--json"],
        ),
        ["ossctl", "contract", "validate"] => (
            "Validate the current repository contract",
            &["ossctl", "contract", "validate", "--json"],
        ),
        ["ossctl", "facts"] => (
            "Detect facts for the current repository",
            &["ossctl", "facts", "--json"],
        ),
        ["ossctl", "audit"] => (
            "Audit the current repository",
            &["ossctl", "audit", "--json"],
        ),
        ["ossctl", "release"] | ["ossctl", "release", "list"] => (
            "List release runs",
            &["ossctl", "release", "list", "--json"],
        ),
        ["ossctl", "release", "plan"] => (
            "Seal a patch release plan",
            &["ossctl", "release", "plan", "--bump", "patch", "--json"],
        ),
        ["ossctl", "release", "cut"] => (
            "Execute an approved plan",
            &["ossctl", "release", "cut", "--plan", "PLAN_ID", "--json"],
        ),
        ["ossctl", "release", "resume"] => (
            "Resume an interrupted release",
            &["ossctl", "release", "resume", "RUN_ID", "--json"],
        ),
        ["ossctl", "release", "verify"] => (
            "Verify a release against its destinations",
            &["ossctl", "release", "verify", "RUN_ID", "--json"],
        ),
        ["ossctl", "release", "show"] => (
            "Inspect release progress",
            &["ossctl", "release", "show", "RUN_ID", "--json"],
        ),
        ["ossctl", "release", "abandon"] => (
            "Abandon a release run",
            &[
                "ossctl",
                "release",
                "abandon",
                "RUN_ID",
                "--reason",
                "superseded",
                "--json",
            ],
        ),
        ["ossctl", "dist"] | ["ossctl", "dist", "generate"] => (
            "Generate distribution infrastructure",
            &["ossctl", "dist", "generate", "--json"],
        ),
        ["ossctl", "skill"] | ["ossctl", "skill", "list"] => (
            "List bundled companion skills",
            &["ossctl", "skill", "list", "--json"],
        ),
        ["ossctl", "skill", "install"] => (
            "Install the release orchestrator skill",
            &["ossctl", "skill", "install", "oss-release", "--json"],
        ),
        ["ossctl", "skill", "print"] => (
            "Print the release orchestrator skill",
            &["ossctl", "skill", "print", "oss-release", "--json"],
        ),
        ["ossctl", "doctor"] => (
            "Run all self-diagnostic checks",
            &["ossctl", "doctor", "--json"],
        ),
        ["ossctl", "version"] => (
            "Inspect version and build provenance",
            &["ossctl", "version", "--json"],
        ),
        _ => return None,
    };
    Some(vec![Example {
        description,
        argv: argv.to_vec(),
    }])
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn every_real_command_has_a_parseable_structured_example() {
        let mut root = Cli::command();
        root.build();
        assert_examples_cover_tree(&root, &[root.get_name().to_string()]);
    }

    fn assert_examples_cover_tree(command: &Command, path: &[String]) {
        let command_examples =
            examples(path).unwrap_or_else(|| panic!("missing examples for {}", path.join(" ")));
        let path_refs: Vec<&str> = path.iter().map(String::as_str).collect();
        for example in command_examples {
            assert!(
                example.argv.starts_with(&path_refs),
                "example does not target {}: {:?}",
                path.join(" "),
                example.argv
            );
            Cli::try_parse_from(&example.argv)
                .unwrap_or_else(|error| panic!("invalid example for {}: {error}", path.join(" ")));
        }
        for subcommand in command.get_subcommands() {
            let mut child_path = path.to_vec();
            child_path.push(subcommand.get_name().to_string());
            assert_examples_cover_tree(subcommand, &child_path);
        }
    }

    #[test]
    fn alias_selection_uses_the_canonical_command_path() {
        let mut root = Command::new("ossctl").subcommand(Command::new("list").alias("ls"));
        root.build();
        let argv = [OsString::from("ossctl"), OsString::from("ls")];
        let (command, path) = selected_command(&root, &argv);
        assert_eq!(command.get_name(), "list");
        assert_eq!(path, ["ossctl", "list"]);
    }

    #[test]
    fn commands_with_subcommands_have_no_value_taking_options() {
        fn walk(command: &Command) {
            if command.get_subcommands().next().is_some() {
                for arg in command.get_arguments() {
                    assert!(
                        !arg.get_action().takes_values(),
                        "{} option {} takes a value and would make structured help selection ambiguous",
                        command.get_name(),
                        arg.get_id()
                    );
                }
            }
            command.get_subcommands().for_each(walk);
        }

        let mut root = Cli::command();
        root.build();
        walk(&root);
    }
}
