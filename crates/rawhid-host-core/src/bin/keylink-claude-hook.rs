fn main() -> std::process::ExitCode {
    rawhid_host_core::run_claude_hook_helper(std::env::args_os().skip(1))
}
