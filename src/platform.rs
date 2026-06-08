use std::io::{self, IsTerminal, Write};

/// Windows 双击 exe 时控制台会随进程退出立刻关闭；在交互场景暂停以便用户看到输出。
pub fn pause_before_exit_if_needed(exit_code: i32) {
    if !should_pause() {
        return;
    }

    let _ = writeln!(io::stderr());
    if exit_code == 0 {
        let _ = writeln!(io::stderr(), "按 Enter 键退出…");
    } else {
        let _ = writeln!(io::stderr(), "发生错误，按 Enter 键退出…");
    }
    let _ = io::stderr().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
}

fn should_pause() -> bool {
    if !cfg!(windows) {
        return false;
    }
    if std::env::var("MPTEXT_NO_PAUSE").is_ok() {
        return false;
    }
    if std::env::var("CI").is_ok() {
        return false;
    }
    // 有管道/重定向时不暂停
    if !io::stdin().is_terminal() {
        return false;
    }
    true
}

pub fn print_welcome() {
    println!("mptext-cli — mptext.top 公众号文章下载工具");
    println!();
    println!("首次使用请先配置 API 密钥（在 https://down.mptext.top 登录后获取）：");
    println!("  mptext config set-token <你的密钥>");
    println!("  mptext auth");
    println!();
    println!("常用命令：");
    println!("  mptext search \"公众号名\"");
    println!("  mptext download \"https://mp.weixin.qq.com/s/xxx\" -o article.md");
    println!("  mptext fetch --fakeid <fakeid> --limit 10");
    println!();
    println!("完整帮助: mptext --help");
}
