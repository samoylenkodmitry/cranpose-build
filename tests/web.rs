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
    // record, it leaves its first screen's records in a global, and on WebGL
    // the stages it built them with. On its own backend it draws on WebGPU
    // (5) at the desktop's viewport and on WebGL (4) at the phone's, as on a
    // Chrome without WebGPU.
    fs::write(
        root.path().join("index.html"),
        "<!doctype html><meta name=\"viewport\" content=\"width=device-width\"><script>\
         if (globalThis.__cranposeRecordFirstScreen) {\
           const desktop = innerWidth > 800;\
           const backend = location.search === '?backend=gl' || !desktop ? 4 : 5;\
           if (backend === 4) globalThis.__cranposeFirstScreenStages = desktop ? 'shape|vs|fs|WIDE=1\\nblit|vs|fs|\\n' : 'blit|vs|fs|\\n';\
           setTimeout(() => { globalThis.__cranposeFirstScreenPipelines = new Uint8Array([7, backend, desktop ? 2 : 1]); }, 50);\
         }</script>",
    )?;
    let mut lines = Vec::new();
    let first_screen = web::record_first_screen(
        root.path(),
        "index.html",
        &Cancellation::default(),
        |line| lines.push(line.to_owned()),
    )?;
    assert_eq!(
        first_screen.pipelines,
        [7, 4, 1, 7, 4, 2, 7, 5, 2],
        "a phone's set once, then a desktop's on WebGL and on WebGPU"
    );
    assert_eq!(
        first_screen.stage_list(),
        "blit|vs|fs|\nshape|vs|fs|WIDE=1\n",
        "the WebGL stages of every run, each once"
    );
    assert_eq!(lines.len(), 4);
    Ok(())
}

/// An app whose web feature brings in a stand-in for Cranpose's
/// `cranpose-gl-templates`: its `cranpose-first-screen-glsl` writes how many
/// stages it read.
fn glsl_writer_fixture(root: &Path) -> Result<()> {
    web_fixture(root)?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='1.0.0'\nedition='2024'\n\n[lib]\ncrate-type=['cdylib','rlib']\n\n[features]\ndefault=['desktop']\ndesktop=[]\nweb=['dep:cranpose-gl-templates']\nrenderer-wgpu=[]\n\n[dependencies]\ncranpose-gl-templates={path='gl templates',optional=true}\n",
    )?;
    let templates = root.join("gl templates");
    fs::create_dir_all(templates.join("src/bin"))?;
    fs::write(
        templates.join("Cargo.toml"),
        "[package]\nname='cranpose-gl-templates'\nversion='1.0.0'\nedition='2024'\n",
    )?;
    fs::write(templates.join("src/lib.rs"), "")?;
    fs::write(
        templates.join("src/bin/cranpose-first-screen-glsl.rs"),
        "fn main() {\
           let args: Vec<String> = std::env::args().skip(1).collect();\
           let stages = std::fs::read_to_string(&args[0]).expect(\"stages\");\
           std::fs::write(&args[1], format!(\"{} stages\", stages.lines().count())).expect(\"out\");\
         }\n",
    )?;
    let status = std::process::Command::new(cranpose_build::tools::tool("cargo")?)
        .args(["generate-lockfile", "--offline"])
        .current_dir(&templates)
        .status()?;
    assert!(status.success());
    Ok(())
}

#[test]
fn the_glsl_writer_is_the_one_the_web_build_resolves() -> Result<()> {
    let root = tempfile::Builder::new()
        .prefix("cranpose glsl ")
        .tempdir()?;
    glsl_writer_fixture(root.path())?;
    let manifest = root.path().join("Cargo.toml");
    let cancel = Cancellation::default();
    let web_build: Vec<String> = ["--offline", "--no-default-features", "--features", "web"]
        .map(String::from)
        .into();
    let writer = web::GlslWriter::find(&manifest, &web_build, &cancel)?
        .expect("the web build resolves the writer");
    let stages = root.path().join("first screen stages.txt");
    fs::write(&stages, "blit|vs|fs|\nshape|vs|fs|WIDE=1\n")?;
    let out = root.path().join("pkg dir/first_screen_glsl.txt");
    fs::create_dir_all(out.parent().expect("pkg dir"))?;
    writer.write(
        &stages,
        &out,
        &root.path().join("target/glsl writer"),
        &cancel,
        |_| {},
    )?;
    assert_eq!(fs::read_to_string(&out)?, "2 stages");
    assert!(
        web::GlslWriter::find(&manifest, &["--offline".into()], &cancel)?.is_none(),
        "the desktop build resolves no writer"
    );
    Ok(())
}
