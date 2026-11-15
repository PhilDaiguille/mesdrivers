//! Vérification des versions dans le Catalogue Microsoft Update (seconde source, lecture seule).
use crate::model::{Catalog, cmp_version};
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

const SEARCH: &str = "https://www.catalog.update.microsoft.com/Search.aspx?q=";

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(concat!("mesdrivers/", env!("CARGO_PKG_VERSION")))
        // TLS natif (schannel sous Windows) : magasin de certificats du système, pas de crypto C à compiler.
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
}

pub fn search_url(query: &str) -> String {
    let enc: String = query
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("{SEARCH}{enc}")
}

/// Requêtes à essayer, de la plus précise à la plus large :
/// sans `REV_`, puis réduit au couple fabricant/modèle (3 segments pour HDAUDIO qui commence par FUNC_).
pub fn queries(hwid: &str) -> Vec<String> {
    let segs: Vec<&str> = hwid.split('&').filter(|s| !s.to_ascii_uppercase().starts_with("REV_")).collect();
    let base = if hwid.to_ascii_uppercase().starts_with("HDAUDIO\\") { 3 } else { 2 };
    let mut out = vec![segs.join("&")];
    if segs.len() > base {
        out.push(segs[..base].join("&"));
    }
    out.retain(|q| !q.is_empty());
    out
}

/// Périphériques qui valent une requête : vrai matériel, pilote non Microsoft.
pub fn worth_checking(hwid: &str, provider: &str) -> bool {
    let h = hwid.to_ascii_uppercase();
    ["PCI\\", "USB\\", "HDAUDIO\\"].iter().any(|b| h.starts_with(b)) && !provider.eq_ignore_ascii_case("Microsoft")
}

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub products: String,
    pub title: String,
    pub date: String,
    pub version: String,
}

/// Lit le tableau de résultats : chaque ligne `<tr id="{guid}_R{n}">`, cellules `{guid}_C{col}_R{n}`.
pub fn parse(html: &str) -> Vec<Entry> {
    let cell = |id: &str| -> String {
        let Some(at) = html.find(&format!("id=\"{id}\"")) else { return String::new() };
        let rest = &html[at..];
        let start = rest.find('>').map_or(rest.len(), |i| i + 1);
        let end = rest.find("</td>").unwrap_or(rest.len()).max(start);
        let mut text = String::new();
        let mut in_tag = false;
        for c in rest[start..end].chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => text.push(c),
                _ => {}
            }
        }
        text.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").split_whitespace().collect::<Vec<_>>().join(" ")
    };
    html.match_indices("<tr id=\"")
        .filter_map(|(i, m)| {
            let row = &html[i + m.len()..];
            let row = &row[..row.find('"')?];
            let (guid, n) = row.rsplit_once("_R")?;
            let title = cell(&format!("{guid}_C1_R{n}"));
            let mut version = cell(&format!("{guid}_C5_R{n}"));
            // Les anciennes entrées ont "n/a" : la version est alors en fin de titre ("… - 2.0.19824.0").
            if !looks_like_version(&version) {
                version = title.rsplit(" - ").next().filter(|v| looks_like_version(v)).unwrap_or_default().to_string();
            }
            let e = Entry {
                id: guid.to_string(),
                products: cell(&format!("{guid}_C2_R{n}")),
                title,
                date: us_date(&cell(&format!("{guid}_C4_R{n}"))),
                version,
            };
            (!e.version.is_empty()).then_some(e)
        })
        .collect()
}

fn looks_like_version(s: &str) -> bool {
    s.contains('.') && s.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// "4/12/2026" -> "2026-04-12"
fn us_date(s: &str) -> String {
    match s.split('/').collect::<Vec<_>>()[..] {
        [m, d, y] => format!("{y}-{m:0>2}-{d:0>2}"),
        _ => s.to_string(),
    }
}

/// Plus haute version publiée pour Windows 10/11 (les pilotes Windows 10 tournent sur 11).
/// L'architecture n'est pas dans le tableau : elle est contrôlée dans l'INF après téléchargement.
pub fn best(entries: Vec<Entry>) -> Option<Entry> {
    entries
        .into_iter()
        .filter(|e| e.products.contains("Windows 10") || e.products.contains("Windows 11"))
        .max_by(|a, b| cmp_version(&a.version, &b.version))
}

/// Version la plus récente publiée pour ce matériel.
pub fn check(agent: &ureq::Agent, hwid: &str) -> Catalog {
    for q in queries(hwid) {
        let html = match agent.get(&search_url(&q)).call().and_then(|mut r| r.body_mut().read_to_string()) {
            Ok(h) => h,
            Err(e) => return Catalog::Error(e.to_string()),
        };
        if let Some(best) = best(parse(&html)) {
            return Catalog::Found { id: best.id, version: best.version, date: best.date, title: best.title };
        }
    }
    Catalog::NotFound
}

/// Lit l'URL du .cab et son empreinte SHA-1 (base64) dans la réponse de DownloadDialog.aspx.
pub fn parse_download(js: &str) -> Option<(String, String)> {
    let get = |key: &str| {
        let at = js.find(&format!("files[0].{key} = '"))? + key.len() + 13;
        Some(js[at..at + js[at..].find('\'')?].to_string())
    };
    Some((get("url")?, get("digest")?))
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// URL HTTPS du .cab et son empreinte SHA-1 (base64) pour une mise à jour du Catalogue.
pub fn download_info(agent: &ureq::Agent, id: &str) -> Result<(String, String), String> {
    let ids = format!(r#"[{{"size":0,"languages":"","uidInfo":"{id}","updateID":"{id}"}}]"#);
    let js = agent
        .post("https://www.catalog.update.microsoft.com/DownloadDialog.aspx")
        .send_form([("updateIDs", ids.as_str())])
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| format!("Catalogue : {e}"))?;
    let (url, digest) = parse_download(&js).ok_or("Catalogue : lien de téléchargement introuvable")?;
    if !url.starts_with("https://") {
        return Err(format!("Catalogue : lien refusé (pas HTTPS) {url}"));
    }
    Ok((url, digest))
}

/// Télécharge le .cab dans `dir` et vérifie son SHA-1 contre l'empreinte du Catalogue.
pub fn download(agent: &ureq::Agent, url: &str, digest: &str, dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let name = url.rsplit('/').next().filter(|n| n.ends_with(".cab")).unwrap_or("pilote.cab");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let mut resp = agent.get(url).call().map_err(|e| format!("Téléchargement : {e}"))?;
    let mut reader = resp.body_mut().with_config().limit(4 << 30).reader();
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut sha = sha1_smol::Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf).map_err(|e| format!("Téléchargement : {e}"))?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| e.to_string())?;
    }
    if base64(&sha.digest().bytes()) != digest {
        let _ = std::fs::remove_file(&path);
        return Err("Fichier corrompu : l'empreinte SHA-1 ne correspond pas au Catalogue.".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Extrait réel d'une page de résultats (novembre 2026).
    const ROW: &str = r#"<tr id="91bf41cb-0052-454a-a3cb-13ff62a706a4_R0">
		<td class="resultsbottomBorder resultspadding resultsIconWidth " id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C0_R0">
                    </td><td class="resultsbottomBorder resultspadding" id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C1_R0">
                        <a id='91bf41cb-0052-454a-a3cb-13ff62a706a4_link' href= "javascript:void(0);" class="contentTextItemSpacerNoBreakLink">
                            Intel net Driver Update (24.40.0.4)
                        </a>
                    </td><td class="resultsbottomBorder resultspadding" id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C2_R0">
                        Windows 10,  Vibranium and later
                    </td><td class="resultsbottomBorder resultspadding" id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C3_R0">
                        Drivers (Networking)
                    </td><td class="resultsbottomBorder resultspadding " id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C4_R0">
                        4/12/2026
                    </td><td class="resultsbottomBorder resultspadding" id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C5_R0">
                        24.40.0.4
                    </td></tr>
	<tr id="91bf41cb-0052-454a-a3cb-13ff62a706a4_R1"><td id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C1_R1">Old &amp; Co</td>
    <td id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C4_R1">12/1/2024</td><td id="91bf41cb-0052-454a-a3cb-13ff62a706a4_C5_R1">23.1.0.1</td></tr>"#;

    #[test]
    fn parses_rows() {
        let rows = parse(ROW);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "91bf41cb-0052-454a-a3cb-13ff62a706a4");
        assert_eq!(rows[0].title, "Intel net Driver Update (24.40.0.4)");
        assert_eq!(rows[0].products, "Windows 10, Vibranium and later");
        assert_eq!((rows[0].date.as_str(), rows[0].version.as_str()), ("2026-04-12", "24.40.0.4"));
        // R1 n'a pas de produits Windows 10/11 : écartée, R0 reste la meilleure.
        assert_eq!(best(parse(ROW)).map(|e| e.version), Some("24.40.0.4".into()));
        assert_eq!(rows[1].title, "Old & Co");
        assert_eq!(rows[1].date, "2024-12-01");
        assert!(parse("<html>aucun résultat</html>").is_empty());

        // Ancienne entrée vue en vrai sur le runner GitHub : version "n/a", reprise depuis le titre.
        let old = r#"<tr id="g_R0"><td id="g_C1_R0">Mellanox Technologies Ltd. - Net - 7/2/2018 12:00:00 AM - 2.0.19824.0</td>
            <td id="g_C4_R0">7/1/2018</td><td id="g_C5_R0">n/a</td></tr>
            <tr id="g_R1"><td id="g_C1_R1">Sans version</td><td id="g_C4_R1">7/1/2018</td><td id="g_C5_R1">n/a</td></tr>"#;
        let rows = parse(old);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, "2.0.19824.0");
    }

    #[test]
    fn parses_download_info() {
        let js = "downloadInformation[0].files[0].url = 'https://catalog.s.download.windowsupdate.com/d/x.cab';\n\
                  downloadInformation[0].files[0].digest = 'Knt//8a9N/UJPGoCYfmupFrEdfQ=';";
        assert_eq!(
            parse_download(js),
            Some(("https://catalog.s.download.windowsupdate.com/d/x.cab".into(), "Knt//8a9N/UJPGoCYfmupFrEdfQ=".into()))
        );
        assert_eq!(parse_download("rien"), None);
        assert_eq!(base64(&sha1_smol::Sha1::from("abc").digest().bytes()), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn builds_queries() {
        assert_eq!(queries(r"PCI\VEN_8086&DEV_51F1&SUBSYS_00948086&REV_01"), [r"PCI\VEN_8086&DEV_51F1&SUBSYS_00948086", r"PCI\VEN_8086&DEV_51F1"]);
        assert_eq!(queries(r"USB\VID_046D&PID_C52B&REV_1211"), [r"USB\VID_046D&PID_C52B"]);
        assert_eq!(queries(r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0257&SUBSYS_17AA2283&REV_1000"), [r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0257&SUBSYS_17AA2283", r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0257"]);
        assert!(queries("").is_empty());
        assert_eq!(search_url(r"PCI\VEN_8086&DEV_51F1"), "https://www.catalog.update.microsoft.com/Search.aspx?q=PCI%5CVEN_8086%26DEV_51F1");
        assert!(worth_checking(r"PCI\VEN_8086", "Intel"));
        assert!(!worth_checking(r"PCI\VEN_8086", "Microsoft"));
        assert!(!worth_checking(r"ROOT\VOLMGR", "Intel"));
    }

    /// Réseau réel : `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn live_download_verifies_sha1() {
        let agent = agent();
        let Catalog::Found { id, .. } = check(&agent, r"PCI\VEN_8086&DEV_51F1") else { panic!("rien trouvé") };
        let dir = std::env::temp_dir().join("mesdrivers-test");
        let (url, digest) = download_info(&agent, &id).expect("info");
        let cab = download(&agent, &url, &digest, &dir).expect("téléchargement");
        assert!(std::fs::metadata(&cab).unwrap().len() > 1_000_000);
        let _ = std::fs::remove_dir_all(dir);
    }
}

