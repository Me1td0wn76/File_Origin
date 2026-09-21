//! 検索。`fo search` / `fo where` と GUI の検索欄が使う。
//!
//! ストアの `search()` は「与えられた条件をそのまま当てる」だけ。
//! ここでは UI が渡してくる曖昧な入力を、ストアが期待する形に整える。

use fo_core::model::{Digest, SearchHit, SearchQuery};
use fo_store::Store;

use crate::Result;

/// 条件を整えてから検索する。
///
/// - `name` にワイルドカードが無ければ部分一致にする。`fo search --name setup` で
///   `setup.zip` が出てこないのは、ユーザーの期待に反する。
/// - `host` は先頭の `www.` を落とさない。落とすと `www.example.com` だけを
///   探したい人が困る。サブドメイン一致はストア側で既に効いている。
pub fn search(store: &Store, mut q: SearchQuery) -> Result<Vec<SearchHit>> {
    if let Some(name) = q.name.as_deref() {
        if !name.contains(['*', '?']) {
            q.name = Some(format!("*{name}*"));
        }
    }
    Ok(store.search(&q)?)
}

/// `fo where <query>` — 「あのファイルはどこ？」に答える。
///
/// 引数が SHA-256（64 桁の 16 進）ならハッシュで、そうでなければファイル名で引く。
/// ハッシュで引けるのは、ダウンロードページに書かれたチェックサムから
/// 手元のファイルを探す用途があるため。
pub fn locate(store: &Store, query: &str) -> Result<Vec<SearchHit>> {
    let q = if looks_like_sha256(query) {
        SearchQuery {
            sha256: Some(Digest(query.to_ascii_lowercase())),
            ..Default::default()
        }
    } else {
        SearchQuery {
            name: Some(query.to_string()),
            ..Default::default()
        }
    };
    search(store, q)
}

fn looks_like_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_detection() {
        assert!(looks_like_sha256(&"a".repeat(64)));
        assert!(looks_like_sha256(&"F".repeat(64)));
        assert!(!looks_like_sha256(&"a".repeat(63)));
        assert!(!looks_like_sha256("setup.zip"));
        assert!(!looks_like_sha256(&"g".repeat(64)));
    }
}
