//! Running a command the user asked for, in the user's own terminal.
//!
//! The doctor can tell someone that installing TLP needs `sudo pacman -S tlp tlp-pd`,
//! but a command you have to copy, find a terminal for, and paste is a command most
//! people do not run. So the application opens a terminal with it instead, and
//! backs off to the clipboard when there is no terminal it recognises.
//!
//! Two rules keep this from becoming a remote shell for the frontend:
//!
//! 1. The command is checked against a closed character set. No `;`, `&&`, `|`,
//!    `$`, backtick or quote can appear, so the string that reaches `bash -lc`
//!    cannot be more than the single command it looks like.
//! 2. The terminal is chosen from a fixed list, and the command is passed as one
//!    argv element rather than interpolated into a shell string.

use std::process::Command;

use crate::battery::ApiResponse;

/// Terminals we know how to hand a command to, in preference order.
///
/// The first entry is the cross-desktop standard; the rest are the terminals a
/// desktop actually ships. `ghostty`, `kitty` and `foot` take the command directly.
pub const TERMINALS: &[(&str, &[&str])] = &[
    ("xdg-terminal-exec", &[]),
    ("ghostty", &["-e"]),
    ("ptyxis", &["--"]),
    ("kgx", &["--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("alacritty", &["-e"]),
    ("kitty", &[]),
    ("foot", &[]),
    ("wezterm", &["start", "--"]),
    ("xterm", &["-e"]),
];

/// Sections shown after the command finishes, so the output is readable and a
/// sudo failure does not take the window with it.
const KEEP_OPEN: &str = "\n\u{2014} finished. Press Enter to close \u{2014}\n";

/// The characters a command may use: a program, its arguments, and the paths and
/// flags they are made of. Everything a shell would treat as syntax is excluded.
pub fn validate_command(command: &str) -> Result<(), String> {
    if command.trim() != command || command.is_empty() {
        return Err("the command is empty or padded with whitespace".to_string());
    }
    if !command
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " ._+/=-@:,".contains(c))
    {
        return Err(
            "the command contains a character a shell would act on; only letters, digits, spaces and . _ + / = - @ : , are allowed"
                .to_string(),
        );
    }
    Ok(())
}

/// First terminal present on this machine, along with the arguments that make it
/// run a command.
pub fn terminal_for(
    present: &dyn Fn(&str) -> bool,
) -> Option<(&'static str, &'static [&'static str])> {
    TERMINALS
        .iter()
        .copied()
        .find(|(binary, _)| present(binary))
}

fn which(binary: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .any(|directory| std::path::Path::new(directory).join(binary).is_file())
}

/// Open a terminal running `command`, keeping the window open afterwards.
///
/// Returns the terminal that was used so the caller can say so.
#[tauri::command]
pub async fn open_terminal_with(command: String) -> ApiResponse<String> {
    if let Err(reason) = validate_command(&command) {
        return ApiResponse {
            success: false,
            data: None,
            error: Some(reason),
        };
    }

    let Some((binary, prefix)) = terminal_for(&which) else {
        return ApiResponse {
            success: false,
            data: None,
            error: Some(
                "No terminal emulator was found, so the command was copied to the clipboard instead."
                    .to_string(),
            ),
        };
    };

    let script = format!("{}\nprintf '{}'\nread -r _", command, KEEP_OPEN);
    let mut invocation = Command::new(binary);
    invocation.args(prefix);
    invocation.arg("bash").arg("-lc").arg(script);

    match invocation.spawn() {
        Ok(_) => ApiResponse {
            success: true,
            data: Some(format!("Opened in {}", binary)),
            error: None,
        },
        Err(error) => ApiResponse {
            success: false,
            data: None,
            error: Some(format!("Could not start {}: {}", binary, error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_install_command_passes_the_guard() {
        for command in [
            "sudo pacman -S tlp tlp-pd",
            "sudo apt install tlp tlp-rdw",
            "sudo dnf install tlp tlp-rdw",
            "sudo modprobe -r thinkpad_acpi && sudo modprobe thinkpad_acpi",
        ] {
            let result = validate_command(command);
            // The last one is deliberately not allowed to pass: it chains commands.
            if command.contains("&&") {
                assert!(result.is_err(), "{command} should be refused");
            } else {
                assert!(result.is_ok(), "{command} was refused: {result:?}");
            }
        }
    }

    #[test]
    fn shell_syntax_is_refused() {
        for hostile in [
            "sudo pacman -S tlp; rm -rf /",
            "sudo pacman -S tlp && reboot",
            "sudo pacman -S tlp | tee /etc/passwd",
            "sudo pacman -S $(cat /etc/shadow)",
            "sudo pacman -S `id`",
            "sudo pacman -S \"tlp\"",
            "sudo pacman -S tlp\nreboot",
            "",
            "  sudo pacman -S tlp  ",
        ] {
            assert!(
                validate_command(hostile).is_err(),
                "{hostile:?} was accepted"
            );
        }
    }

    #[test]
    fn the_standard_terminal_wins_over_the_desktop_specific_ones() {
        let found =
            terminal_for(&|binary| matches!(binary, "gnome-terminal" | "xdg-terminal-exec"))
                .expect("a terminal");
        assert_eq!(found.0, "xdg-terminal-exec");
        assert!(found.1.is_empty());
    }

    #[test]
    fn ghostty_is_used_with_its_execute_flag() {
        let found = terminal_for(&|binary| binary == "ghostty").expect("ghostty");
        assert_eq!(found.0, "ghostty");
        assert_eq!(found.1, &["-e"]);
    }

    #[test]
    fn no_terminal_is_reported_rather_than_guessed() {
        assert!(terminal_for(&|_| false).is_none());
    }
}
