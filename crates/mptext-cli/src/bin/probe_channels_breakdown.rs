//! 临时诊断：手动复现 list_all_albums 的轮次，逐步打印。
//! 用完即删。

use anyhow::Result;

const BIZ: &str = "MzIwMDY3MTE2NQ==";
const ALBUM_A: &str = "4352921130605740036";

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = mptext_core::WechatClient::new(mptext_core::MpSession::default())?;
    let some = client.list_articles_album(BIZ, ALBUM_A).await.unwrap_or_default();
    let seed = some.first().expect("合集 A 非空").url.clone();

    let mut album_ids: Vec<String> = Vec::new();
    let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut articles: Vec<mptext_core::ArticleItem> = Vec::new();
    let mut scanned: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pending: Vec<String> = vec![seed];

    for round in 0..4 {
        println!("=== 第 {} 轮，待扫种子 {} 篇 ===", round + 1, pending.len());
        let mut new_albums = 0usize;
        let mut n = 0usize;
        for s in &pending {
            if !scanned.insert(s.clone()) {
                continue;
            }
            n += 1;
            let before = album_ids.len();
            for id in client.discover_album_ids(s).await {
                if !album_ids.contains(&id) {
                    album_ids.push(id);
                }
            }
            new_albums += album_ids.len() - before;
            if n % 20 == 0 {
                println!("  已扫 {n} 篇，合集 {} 个 {:?}", album_ids.len(), album_ids);
            }
            if scanned.len() >= 300 {
                break;
            }
        }
        println!("  本轮扫了 {n} 篇，新增合集 {new_albums} 个，合集总数 {}", album_ids.len());
        if new_albums == 0 {
            println!("  没有新合集，停");
            break;
        }
        let before_len = articles.len();
        for album_id in album_ids.clone() {
            let list = client.list_articles_album(BIZ, &album_id).await.unwrap_or_default();
            for a in list {
                if seen_urls.insert(a.url.clone()) {
                    articles.push(a);
                }
            }
            println!("  拉完 {album_id}: 累计 {} 篇", articles.len());
        }
        pending = articles.iter().skip(before_len).map(|a| a.url.clone()).collect();
        println!("  下一轮种子 {} 篇", pending.len());
    }
    println!("\n最终 {} 篇", articles.len());
    Ok(())
}
