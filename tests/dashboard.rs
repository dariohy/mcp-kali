#[test]
fn dashboard_uses_requested_accents_and_navigable_catalogs() {
    let dashboard = include_str!("../src/dashboard.html");

    assert!(dashboard.contains("--accent-green:#3CB335"));
    assert!(dashboard.contains("--accent-blue:#006DC7"));
    assert!(dashboard.contains("id=\"tool-search\""));
    assert!(dashboard.contains("id=\"tool-plugin-filter\""));
    assert!(dashboard.contains("class=\"runtime-card plugin-card\""));
    assert!(dashboard.contains("id=\"reference-search\""));
    assert!(dashboard.contains("id=\"reference-plugin-filter\""));
    assert!(dashboard.contains("class=\"reference-index\""));
}
