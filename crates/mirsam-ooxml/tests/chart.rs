//! Chart text containers: does the adapter find the strings a chart draws,
//! and attribute each set to the container that actually draws it?
//!
//! Getting the attribution wrong is the failure that matters here. A finding
//! on strings a container does not draw is a false positive on text the
//! reviewer cannot even find, so every case below fixes which strings belong
//! to which container.

use mirsam_core::{Direction, Engine, Resolved, UnitKind};
use mirsam_ooxml::pptx::scan_xml;

const NS: &str = r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

const PART: &str = "ppt/charts/chart1.xml";

fn chart(inner: &str) -> String {
    format!(r#"<c:chartSpace {NS}><c:chart>{inner}</c:chart></c:chartSpace>"#)
}

fn str_cache(values: &[&str]) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:strCache><c:ptCount val="{}"/>{points}</c:strCache>"#,
        values.len()
    )
}

fn series(name: &str, categories: &[&str], values: &[i32]) -> String {
    let numbers: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
        .collect();
    format!(
        r#"<c:ser><c:idx val="0"/><c:tx><c:strRef><c:f>S!$A$1</c:f>{}</c:strRef></c:tx><c:cat><c:strRef><c:f>S!$A$2</c:f>{}</c:strRef></c:cat><c:val><c:numRef><c:f>S!$B$2</c:f><c:numCache><c:ptCount val="{}"/>{numbers}</c:numCache></c:numRef></c:val></c:ser>"#,
        str_cache(&[name]),
        str_cache(categories),
        values.len(),
    )
}

const QUARTERS: [&str; 2] = ["الربع الأول", "الربع الثاني"];
const AX: &str = "111111111";
const VAL_AX: &str = "222222222";

/// A bar chart with one Arabic series over Arabic categories, plus whatever
/// axis, legend or label markup the test is about.
fn bar_chart(cat_ax: &str, extra: &str) -> String {
    chart(&format!(
        r#"<c:plotArea><c:layout/><c:barChart><c:barDir val="col"/>{}{extra}<c:axId val="{AX}"/><c:axId val="{VAL_AX}"/></c:barChart><c:catAx><c:axId val="{AX}"/><c:axPos val="b"/>{cat_ax}<c:crossAx val="{VAL_AX}"/></c:catAx><c:valAx><c:axId val="{VAL_AX}"/><c:axPos val="l"/><c:crossAx val="{AX}"/></c:valAx></c:plotArea>"#,
        series("الإيرادات", &QUARTERS, &[18, 21]),
    ))
}

fn units_of_kind(xml: &str, kind: UnitKind) -> Vec<mirsam_core::TextUnit> {
    scan_xml(PART, xml)
        .unwrap()
        .into_iter()
        .filter(|u| u.kind == kind)
        .collect()
}

#[test]
fn a_category_axis_draws_the_categories_of_the_chart_that_names_it() {
    let units = units_of_kind(&bar_chart("", ""), UnitKind::ChartText);
    assert_eq!(units.len(), 1, "{units:#?}");
    assert_eq!(units[0].id.0, "ppt/charts/chart1.xml#catax1");
    assert_eq!(units[0].text, QUARTERS.join("\n"));
    assert_eq!(units[0].props.direction, Resolved::Unset);
    assert_eq!(
        units[0].location.container.as_deref(),
        Some("category axis")
    );
}

#[test]
fn a_value_axis_is_not_a_unit() {
    // It draws formatted numbers, which have no direction to get wrong. The
    // chart above has one, and it is not among the units.
    let units = units_of_kind(&bar_chart("", ""), UnitKind::ChartText);
    assert!(
        units.iter().all(|u| !u.id.0.contains("valax")),
        "{units:#?}"
    );
}

#[test]
fn an_axis_direction_the_chart_declares_is_explicit() {
    let with_direction = r#"<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/></a:p></c:txPr>"#;
    let units = units_of_kind(&bar_chart(with_direction, ""), UnitKind::ChartText);
    assert_eq!(units[0].props.direction, Resolved::Explicit(Direction::Rtl));
    assert!(
        Engine::with_default_rules()
            .audit(&units)
            .diagnostics
            .is_empty()
    );
}

#[test]
fn an_axis_titles_text_properties_are_not_the_axiss() {
    // c:title has a c:txPr of its own, and a direction on it governs the
    // title, not the labels. Reading it as the axis's would silence a real
    // finding — the failure mode this whole module exists to avoid.
    let titled = r#"<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/><a:r><a:t>المحور</a:t></a:r></a:p></c:rich></c:tx><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/></a:p></c:txPr></c:title>"#;
    let units = units_of_kind(&bar_chart(titled, ""), UnitKind::ChartText);
    assert_eq!(units.len(), 1, "{units:#?}");
    assert_eq!(units[0].props.direction, Resolved::Unset, "{units:#?}");
}

#[test]
fn a_legend_draws_the_series_names() {
    let units = units_of_kind(
        &chart(&format!(
            r#"<c:plotArea><c:barChart>{}<c:axId val="{AX}"/><c:axId val="{VAL_AX}"/></c:barChart></c:plotArea><c:legend><c:legendPos val="r"/></c:legend>"#,
            series("الإيرادات", &QUARTERS, &[18, 21]),
        )),
        UnitKind::ChartText,
    );
    assert_eq!(units.len(), 1, "{units:#?}");
    assert_eq!(units[0].id.0, "ppt/charts/chart1.xml#legend1");
    assert_eq!(units[0].text, "الإيرادات");
}

#[test]
fn a_pie_legend_draws_the_categories_instead() {
    // A pie has one series and lists its slices, so the legend shows the
    // category names. Judging it on the series name would judge a string
    // nobody sees.
    let units = units_of_kind(
        &chart(&format!(
            r#"<c:plotArea><c:pieChart>{}</c:pieChart></c:plotArea><c:legend><c:legendPos val="r"/></c:legend>"#,
            series("Revenue", &QUARTERS, &[18, 21]),
        )),
        UnitKind::ChartText,
    );
    assert_eq!(units.len(), 1, "{units:#?}");
    assert_eq!(units[0].text, QUARTERS.join("\n"));
}

#[test]
fn data_labels_draw_only_what_they_are_set_to_show() {
    let showing =
        |flags: &str| format!(r#"<c:dLbls>{flags}<c:showLeaderLines val="1"/></c:dLbls>"#);

    // Values only — numbers, so there is nothing to judge and no unit.
    let values_only =
        showing(r#"<c:showVal val="1"/><c:showCatName val="0"/><c:showSerName val="0"/>"#);
    assert!(
        units_of_kind(&bar_chart("", &values_only), UnitKind::ChartText)
            .iter()
            .all(|u| !u.id.0.contains("dlbls")),
    );

    // Category names — the same strings the axis draws, in a second place
    // that needs its own direction.
    let with_categories = showing(r#"<c:showVal val="0"/><c:showCatName val="1"/>"#);
    let labels: Vec<_> = units_of_kind(&bar_chart("", &with_categories), UnitKind::ChartText)
        .into_iter()
        .filter(|u| u.id.0.contains("dlbls"))
        .collect();
    assert_eq!(labels.len(), 1, "{labels:#?}");
    assert_eq!(labels[0].id.0, "ppt/charts/chart1.xml#dlbls1");
    assert_eq!(labels[0].text, QUARTERS.join("\n"));
    assert_eq!(labels[0].location.container.as_deref(), Some("data labels"));
}

#[test]
fn an_english_chart_is_silent() {
    let english = chart(&format!(
        r#"<c:plotArea><c:barChart>{}<c:axId val="{AX}"/><c:axId val="{VAL_AX}"/></c:barChart><c:catAx><c:axId val="{AX}"/><c:crossAx val="{VAL_AX}"/></c:catAx></c:plotArea><c:legend><c:legendPos val="r"/></c:legend>"#,
        series("Revenue", &["First quarter", "Second quarter"], &[18, 21]),
    ));
    let units = scan_xml(PART, &english).unwrap();
    assert!(
        Engine::with_default_rules()
            .audit(&units)
            .diagnostics
            .is_empty(),
        "{units:#?}"
    );
}

#[test]
fn a_part_that_is_not_a_chart_produces_no_chart_units() {
    let slide = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:bodyPr/><a:p><a:r><a:t>مرحبا</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
    let units = scan_xml("ppt/slides/slide1.xml", slide).unwrap();
    assert!(units.iter().all(|u| u.kind == UnitKind::Paragraph));
}

#[test]
fn a_chart_container_is_judged_by_the_container_rule_alone() {
    // It carries no language, font or alignment of its own; the paragraph
    // rules must not report those as missing on it.
    let units = units_of_kind(&bar_chart("", ""), UnitKind::ChartText);
    let report = Engine::with_default_rules().audit(&units);
    let rules: Vec<_> = report.diagnostics.iter().map(|d| d.rule.0).collect();
    assert_eq!(rules, ["container-direction"], "{report:#?}");
}
