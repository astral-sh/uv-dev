use std::collections::BTreeSet;

use clap::{CommandFactory, ValueHint};

use uv_cli::Cli;

#[test]
fn structured_option_value_hints() {
    let expected = BTreeSet::from([
        "upgrade_package",
        "config_setting",
        "config_settings_package",
        "config_setting_package",
    ]);
    let cli = Cli::command();
    let mut pending = vec![&cli];
    let mut seen = BTreeSet::new();
    while let Some(command) = pending.pop() {
        for argument in command.get_arguments() {
            let name = argument.get_id().as_str();
            if !argument.is_hide_set() && expected.contains(name) {
                assert_eq!(
                    argument.get_value_hint(),
                    ValueHint::Other,
                    "{}: {name}",
                    command.get_name()
                );
                seen.insert(name);
            }
        }
        pending.extend(command.get_subcommands());
    }
    assert_eq!(seen, expected);
}
