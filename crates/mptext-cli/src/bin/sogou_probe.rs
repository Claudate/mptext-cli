//! 一次性实网探针：按指定昵称走搜狗后备，打印解出来的文章地址。用完即删。

use mptext_core::WechatClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = WechatClient::from_saved_session()?;
    let articles = client
        .list_articles("MzkyNTIyNjA2Ng==", "大学生知行计划", 0, 5, None)
        .await?;
    println!("count {}", articles.len());
    for item in &articles {
        println!("{}\t{}", item.title, item.url);
    }
    Ok(())
}
