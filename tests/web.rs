use anyhow::Result;
use cranpose_build::{Cancellation, Platform, Request, engine, web};
use std::{fs, path::Path};

fn web_fixture(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='1.0.0'\nedition='2024'\n\n[lib]\ncrate-type=['cdylib','rlib']\n\n[features]\ndefault=['desktop']\ndesktop=[]\nweb=[]\nrenderer-wgpu=[]\n",
    )?;
    fs::write(root.join("src/lib.rs"), "pub fn start() {}\n")?;
    fs::write(root.join("index.html"), "<!doctype html>\n")?;
    Ok(())
}

#[test]
fn a_web_plan_builds_the_page_module_with_the_web_features() -> Result<()> {
    let root = tempfile::Builder::new().prefix("cranpose web ").tempdir()?;
    web_fixture(root.path())?;
    let mut request = Request::new(root.path().join("Cargo.toml"), Platform::Web);
    request.release = true;
    let plan = engine::plan(&request, &Cancellation::default())?;
    assert_eq!(plan.program, "wasm-pack");
    let arguments = plan.arguments.join(" ");
    assert!(arguments.contains("--target web --release --out-dir"));
    assert!(arguments.contains("-- --locked --no-default-features --features web,renderer-wgpu"));
    assert!(plan.environment.contains_key("CARGO_TARGET_DIR"));
    assert!(
        plan.expected_binary
            .as_ref()
            .is_some_and(|pkg| pkg.ends_with("cranpose-web/release/pkg"))
    );
    Ok(())
}

#[test]
fn the_site_serves_files_under_its_root_only() -> Result<()> {
    let root = tempfile::Builder::new()
        .prefix("cranpose site ")
        .tempdir()?;
    fs::create_dir(root.path().join("pkg dir"))?;
    fs::write(root.path().join("pkg dir/module.wasm"), [0, 97, 115, 109])?;
    fs::write(root.path().join("index.html"), "page")?;
    let site = web::Site::serve(root.path())?;

    let mut response = ureq::get(&site.url("pkg%20dir/module.wasm")).call()?;
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/wasm")
    );
    assert_eq!(response.body_mut().read_to_vec()?, [0, 97, 115, 109]);
    assert_eq!(
        ureq::get(&site.url(""))
            .call()?
            .body_mut()
            .read_to_string()?,
        "page"
    );
    assert!(ureq::get(&site.url("missing.js")).call().is_err());

    assert_eq!(web::served_file(root.path(), "/../secret"), None);
    assert_eq!(
        web::served_file(root.path(), "/pkg%2F..%2F..%2Fsecret"),
        None
    );
    Ok(())
}

#[test]
fn the_recorder_takes_the_first_screen_a_page_leaves_at_each_viewport() -> Result<()> {
    if web::find_chrome().is_none() {
        eprintln!("skipped: no Chrome or Chromium");
        return Ok(());
    }
    let root = tempfile::Builder::new()
        .prefix("cranpose record ")
        .tempdir()?;
    // A page that answers the recorder as a Cranpose page does: when asked to
    // record, it leaves its first screen's records in a global.
    fs::write(
        root.path().join("index.html"),
        "<!doctype html><meta name=\"viewport\" content=\"width=device-width\"><script>\
         if (globalThis.__cranposeRecordFirstScreen && location.search === '?backend=gl') {\
           setTimeout(() => { globalThis.__cranposeFirstScreenPipelines = new Uint8Array([7, 8, innerWidth > 800 ? 2 : 1]); }, 50);\
         }</script>",
    )?;
    let mut lines = Vec::new();
    let records = web::record_first_screen(
        root.path(),
        "index.html",
        &Cancellation::default(),
        |line| lines.push(line.to_owned()),
    )?;
    assert_eq!(
        records,
        [7, 8, 1, 7, 8, 2],
        "a phone's set, then a desktop's"
    );
    assert_eq!(lines.len(), 2);
    Ok(())
}
