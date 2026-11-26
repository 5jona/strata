//! Static curated tool bundles (including modern Shell presets) and bootstrap profiles.

use strata_protocol::rpc::ToolsBootstrapProfile;

pub(super) struct StaticBundle {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) summary: &'static str,
    pub(super) tools: &'static [&'static str],
}

pub(super) const BUNDLES: &[StaticBundle] = &[
    StaticBundle {
        id: "core-dev",
        name: "核心开发套件",
        summary: "Python 3.12、Node.js LTS、uv、git、curl、tmux、ripgrep、fd、jq 一键就绪",
        tools: &[
            "apt:ca-certificates",
            "apt:curl",
            "apt:git",
            "apt:tmux",
            "mise:python@3.12",
            "mise:node@lts",
            "mise:uv@latest",
            "mise:ripgrep@latest",
            "mise:fd@latest",
            "mise:jq@latest",
        ],
    },
    StaticBundle {
        id: "python-uv",
        name: "Python + uv 套件",
        summary: "通过 mise 管理 Python 3.12，搭配 uv、ruff 代码检查与 httpie 调试工具",
        tools: &["mise:python@3.12", "mise:uv@latest", "uv:ruff", "uv:httpie"],
    },
    StaticBundle {
        id: "node-ts",
        name: "Node + Bun 套件",
        summary: "Node.js LTS 与 Bun TypeScript 运行时组合",
        tools: &["mise:node@lts", "mise:bun@latest"],
    },
    StaticBundle {
        id: "debug-l3-l5",
        name: "L3-L5 诊断套件",
        summary: "Android/GKI 容器调试工具：strace、tcpdump、bpftrace 与 sqlite3",
        tools: &["apt:strace", "apt:tcpdump", "apt:bpftrace", "apt:sqlite3"],
    },
    StaticBundle {
        id: "shell-zsh",
        name: "现代 Zsh 体验套件",
        summary: "Zsh + Starship 提示符 + 自动建议/语法高亮 + zoxide + eza + fzf，完全兼容 POSIX",
        tools: &[
            "apt:zsh",
            "apt:zsh-autosuggestions",
            "apt:zsh-syntax-highlighting",
            "mise:starship@latest",
            "mise:zoxide@latest",
            "mise:eza@latest",
            "mise:fzf@latest",
        ],
    },
    StaticBundle {
        id: "shell-fish",
        name: "现代 Fish 体验套件",
        summary: "Fish 智能补全 Shell + Starship + zoxide + eza + fzf，开箱即用免折腾",
        tools: &[
            "apt:fish",
            "mise:starship@latest",
            "mise:zoxide@latest",
            "mise:eza@latest",
            "mise:fzf@latest",
        ],
    },
    StaticBundle {
        id: "shell-nushell",
        name: "Nushell 结构化套件",
        summary: "Nushell + Starship + Carapace 补全 + zoxide + eza + fzf（非 POSIX 语法，适合进阶用户）",
        tools: &[
            "mise:nushell@latest",
            "mise:starship@latest",
            "mise:carapace@latest",
            "mise:zoxide@latest",
            "mise:eza@latest",
            "mise:fzf@latest",
        ],
    },
];

/// Returns APT & mise packages added after bootstrap base init for a given profile.
pub(super) fn profile_packages(
    profile: ToolsBootstrapProfile,
) -> (&'static [&'static str], &'static [&'static str]) {
    match profile {
        ToolsBootstrapProfile::Minimal => {
            (&["ca-certificates", "curl", "git", "xz-utils", "zstd"], &[])
        }
        ToolsBootstrapProfile::Dev => (
            &["ca-certificates", "curl", "git", "xz-utils", "zstd", "tmux"],
            &[
                "python@3.12",
                "node@lts",
                "ripgrep@latest",
                "fd@latest",
                "jq@latest",
            ],
        ),
        ToolsBootstrapProfile::Debug => (
            &[
                "ca-certificates",
                "curl",
                "git",
                "xz-utils",
                "zstd",
                "tmux",
                "strace",
                "tcpdump",
                "bpftrace",
                "sqlite3",
            ],
            &[
                "python@3.12",
                "node@lts",
                "ripgrep@latest",
                "fd@latest",
                "jq@latest",
            ],
        ),
    }
}
