#[test]
fn launcher_resolution_uses_the_installed_apks_activity() {
    let output = "priority=0 preferredOrder=0 match=0x108000\ncom.example.app/dev.cranpose.android.CranposeActivity\n";
    assert_eq!(
        cranpose_build::engine::android_activity(output, "com.example.app").expect("launcher"),
        "com.example.app/dev.cranpose.android.CranposeActivity"
    );
    for invalid in [
        "No activity found",
        "com.other/.Activity",
        "com.example.app/.Activity;reboot",
        "com.example.app/.One\ncom.example.app/.Two",
    ] {
        assert!(cranpose_build::engine::android_activity(invalid, "com.example.app").is_err());
    }
}
