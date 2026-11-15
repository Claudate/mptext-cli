use std::io::{self, IsTerminal, Write};

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
    if !io::stdin().is_terminal() {
        return false;
    }
    true
}

pub fn print_welcome() {
    println!("mptext — 微信公众号文章下载");
    println!();
    println!("首次使用先扫码登录公众号后台（任意一个你有权限的号即可）：");
    println!("  mptext login");
    println!("  mptext auth");
    println!();
    println!("常用命令：");
    println!("  mptext search \"公众号名\"");
    println!("  mptext download \"https://mp.weixin.qq.com/s/xxx\" -o article.md");
    println!("  mptext fetch --fakeid <fakeid> --limit 10");
    println!();
    println!("完整帮助: mptext --help");
    println!();
    println!("提示：也可使用图形界面版本 mptext 桌面应用");
}
