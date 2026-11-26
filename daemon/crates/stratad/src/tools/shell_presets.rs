//! Idempotent post-install shell configuration snippets for modern shell preset bundles.

/// Returns an optional idempotent POSIX shell snippet to run after installing bundle `id`.
pub(super) fn post_install_shell_hook(bundle_id: &str) -> Option<&'static str> {
    match bundle_id {
        "shell-zsh" => Some(ZSH_PRESET_SETUP),
        "shell-fish" => Some(FISH_PRESET_SETUP),
        "shell-nushell" => Some(NUSHELL_PRESET_SETUP),
        _ => None,
    }
}

const ZSH_PRESET_SETUP: &str = r##"
mkdir -p /root/.config
touch /root/.zshrc
if ! grep -Fq "# >>> strata-shell-zsh >>>" /root/.zshrc; then
  cat >> /root/.zshrc <<'STRATA_ZSH_EOF'
# >>> strata-shell-zsh >>>
[ -r /etc/strata/env.zsh ] && source /etc/strata/env.zsh
HISTFILE=/root/.zsh_history
HISTSIZE=10000
SAVEHIST=10000
setopt APPEND_HISTORY SHARE_HISTORY HIST_IGNORE_DUPS AUTO_CD
autoload -Uz compinit && compinit -C
[ -r /usr/share/zsh-autosuggestions/zsh-autosuggestions.zsh ] && source /usr/share/zsh-autosuggestions/zsh-autosuggestions.zsh
[ -r /usr/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh ] && source /usr/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh
command -v starship >/dev/null 2>&1 && eval "$(starship init zsh)"
command -v zoxide >/dev/null 2>&1 && eval "$(zoxide init zsh)"
command -v fzf >/dev/null 2>&1 && source <(fzf --zsh 2>/dev/null) 2>/dev/null || true
command -v eza >/dev/null 2>&1 && alias ls='eza --icons=auto' && alias ll='eza -la --icons=auto'
# <<< strata-shell-zsh <<<
STRATA_ZSH_EOF
fi
"##;

const FISH_PRESET_SETUP: &str = r#"
mkdir -p /root/.config/fish/conf.d
cat > /root/.config/fish/conf.d/10-strata-shell.fish <<'STRATA_FISH_EOF'
# Strata modern Fish preset (managed)
if status is-interactive
    command -q starship; and starship init fish | source
    command -q zoxide; and zoxide init fish | source
    command -q fzf; and fzf --fish 2>/dev/null | source
    if command -q eza
        alias ls='eza --icons=auto'
        alias ll='eza -la --icons=auto'
    end
end
STRATA_FISH_EOF
"#;

const NUSHELL_PRESET_SETUP: &str = r#"
mkdir -p /root/.config/nushell/autoload
cat > /root/.config/nushell/autoload/10-strata-shell.nu <<'STRATA_NU_EOF'
# Strata modern Nushell preset (managed)
$env.PATH = ($env.PATH | split row (char esep) | prepend [
    "/root/.local/share/mise/shims"
    "/root/.local/bin"
    "/root/.cargo/bin"
    "/opt/strata/mise/shims"
    "/opt/strata/bin"
] | uniq)
$env.COLORTERM = "truecolor"
if (which starship | is-not-empty) {
    mkdir ($nu.data-dir | path join "vendor/autoload")
    starship init nu | save -f ($nu.data-dir | path join "vendor/autoload/starship.nu")
}
if (which zoxide | is-not-empty) {
    mkdir ($nu.data-dir | path join "vendor/autoload")
    zoxide init nushell | save -f ($nu.data-dir | path join "vendor/autoload/zoxide.nu")
}
if (which carapace | is-not-empty) {
    mkdir ($nu.data-dir | path join "vendor/autoload")
    carapace _carapace nushell | save -f ($nu.data-dir | path join "vendor/autoload/carapace.nu")
}
STRATA_NU_EOF
"#;
