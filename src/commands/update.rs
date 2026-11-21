use crate::cli::UpdateCmd;

pub fn run(cmd: UpdateCmd) -> i32 {
    let channel = cmd
        .channel
        .or(cmd.channel_flag)
        .unwrap_or_else(|| "stable".to_string());
    if let Some(v) = cmd.version {
        eprintln!(
            "Installing an exact version ({v}) is not supported by this build.\n\
             Upstream releases live at https://download.ante.run/{channel}/."
        );
        return 1;
    }
    eprintln!(
        "This is a clean-room reimplementation and does not self-update.\n\
         Check the installed version with `ante --version`; upstream installs with:\n\
         curl -fsSL https://download.ante.run/install.sh | sh"
    );
    1
}
