//! XFA validation through the CLI/MCP tool table. Fixtures are contributor-original.
use pdfcraft_automation::{Automation, Content};
use pdfcraft_engine::DocId;
use serde_json::{Value, json};

fn call(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(v) => v,
        other => panic!("expected JSON: {other:?}"),
    }
}

const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="application" layout="tb"><pageSet><pageArea><medium short="300pt" long="400pt"/><contentArea x="20pt" y="20pt" w="260pt" h="360pt"/></pageArea></pageSet>
<variables><text name="status">unchecked</text><script name="checks" contentType="application/x-javascript">
function validate(field) {
    var valid = field.rawValue != null &amp;&amp; Number(field.rawValue) &gt;= 1 &amp;&amp; Number(field.rawValue) &lt;= 12;
    var border = field.ui.oneOfChild.border;
    if (valid) { border.parent.nodes.remove(border); status.value = 'valid'; }
    else { border.presence = 'visible'; border.edge.presence = 'visible'; border.edge.color.value = '255,0,0'; border.edge.thickness = '2pt'; status.value = 'invalid'; xfa.host.messageBox('Enter a month between 1 and 12'); }
    return valid;
}
</script></variables>
<field name="month" w="100pt" h="24pt"><ui><textEdit/></ui><validate nullTest="error"><script contentType="application/x-javascript">checks.validate(this);</script></validate></field>
<field name="validate" w="100pt" h="24pt"><ui><button/></ui><caption><value><text>Validate</text></value></caption><event activity="click"><script contentType="application/x-javascript">checks.validate(month);</script></event></field>
<field name="clear" w="100pt" h="24pt"><ui><button/></ui><caption><value><text>Clear</text></value></caption><event activity="click"><script contentType="application/x-javascript">if (xfa.host.messageBox('Clear all entries?', 'Confirm', 2, 1) == 1) { xfa.datasets.data.nodes.remove(xfa.resolveNode('xfa.datasets.data.application')); }</script></event></field>
</subform></template>"#;

fn field(a: &Automation, id: u64, ending: &str) -> String {
    a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name.ends_with(ending)).unwrap().name.clone()
}

#[test]
fn scripted_validation_clear_confirmation_and_undo_survive_save() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(TEMPLATE)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    let month = field(&a, id, "month");
    let validate = field(&a, id, "validate");
    let clear = field(&a, id, "clear");
    let result = call(&mut a, "form_fill", json!({"doc":id,"values":{month.clone():"19"}}));
    assert!(result["errors"].as_array().is_none_or(Vec::is_empty), "{result}");
    let look = a.session().get(DocId(id)).unwrap().field_look(&month).unwrap();
    assert_eq!(look.border, Some([1.0, 0.0, 0.0]));
    assert_eq!(look.width, 2.0);
    let result = call(&mut a, "js_run", json!({"doc":id,"field":validate,"script":""}));
    assert!(result["error"].is_null(), "{result}");
    assert!(result["alerts"].as_array().unwrap().iter().any(|v| v.as_str().is_some_and(|s| s.contains("between 1 and 12"))));
    call(&mut a, "doc_save", json!({"doc":id,"path":"saved.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"saved.pdf"}))["doc"].as_u64().unwrap();
    assert_eq!(a.session().get(DocId(reopened)).unwrap().field_look(&month).unwrap().border, Some([1.0, 0.0, 0.0]));
    let generation = a.session().get(DocId(id)).unwrap().edit_generation();
    let result = call(&mut a, "js_run", json!({"doc":id,"field":clear,"script":""}));
    assert_eq!(result["requests"][0]["confirmation"]["message"], "Clear all entries?");
    assert_eq!(a.session().get(DocId(id)).unwrap().edit_generation(), generation);
    call(&mut a, "js_run", json!({"doc":id,"field":clear,"script":"","dialog_answers":[2]}));
    assert_eq!(a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name == month).unwrap().value, ["19"]);
    let result = call(&mut a, "js_run", json!({"doc":id,"field":clear,"script":"","dialog_answers":[1]}));
    assert!(result["error"].is_null(), "{result}");
    assert!(a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name == month).unwrap().value.iter().all(String::is_empty));
    call(&mut a, "edit_undo", json!({"doc":id}));
    assert_eq!(a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name == month).unwrap().value, ["19"]);
    call(&mut a, "form_fill", json!({"doc":id,"values":{month.clone():"12"}}));
    assert_eq!(a.session().get(DocId(id)).unwrap().field_look(&month).unwrap().width, 0.0);
    assert!(a.call("page_render", &json!({"doc":id,"page":1,"dpi":72})).unwrap().iter().any(|c| matches!(c, Content::Png { .. })));
}

#[test]
fn calculated_master_text_tracks_numeric_picture_values() {
    let template = r##"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form" layout="tb"><pageSet><pageArea name="master"><medium short="300pt" long="400pt"/><contentArea x="20pt" y="50pt" w="260pt" h="300pt"/>
<field name="header" id="headerValue" presence="hidden"><bind match="none"/><ui><textEdit/></ui><calculate><script contentType="application/x-javascript">this.rawValue = 'Reference ' + form.month.rawValue;</script></calculate></field>
<draw x="20pt" y="10pt" w="240pt" h="24pt"><value><exData contentType="text/html"><body xmlns="http://www.w3.org/1999/xhtml"><p><span xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/" xfa:embed="#headerValue"/></p></body></exData></value></draw>
</pageArea></pageSet>
<field name="month" w="100pt" h="24pt"><ui><textEdit/></ui><bind><picture>num{99}</picture></bind><value><text>05</text></value><validate><script contentType="application/x-javascript">if (typeof this.rawValue != 'string' || this.rawValue != '5') xfa.host.messageBox('Unexpected raw value');</script></validate></field>
</subform></template>"##;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let opened = call(&mut a, "doc_open", json!({"path":"form.pdf"}));
    assert!(opened["errors"].as_array().is_none_or(Vec::is_empty), "{opened}");
    let id = opened["doc"].as_u64().unwrap();
    let text = call(&mut a, "text_extract", json!({"doc":id}));
    assert!(text["pages"][0]["text"].as_str().unwrap().contains("Reference 5"), "{text}");
    let result = call(&mut a, "form_fill", json!({"doc":id,"values":{"month":"05"}}));
    assert!(result["errors"].as_array().is_none_or(Vec::is_empty), "{result}");
    assert!(result["alerts"].as_array().is_none_or(Vec::is_empty), "{result}");
    call(&mut a, "form_fill", json!({"doc":id,"values":{"month":"12"}}));
    let text = call(&mut a, "text_extract", json!({"doc":id}));
    assert!(text["pages"][0]["text"].as_str().unwrap().contains("Reference 12"), "{text}");
    call(&mut a, "doc_save", json!({"doc":id,"path":"saved.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"saved.pdf"}))["doc"].as_u64().unwrap();
    let text = call(&mut a, "text_extract", json!({"doc":reopened}));
    assert!(text["pages"][0]["text"].as_str().unwrap().contains("Reference 12"), "{text}");
}

#[test]
fn typing_date_segments_previews_focus_without_saving_a_partial_value() {
    let template = TEMPLATE.replace("<ui><textEdit/></ui>", "<ui><textEdit/></ui><event activity=\"change\"><script contentType=\"application/x-javascript\">if (xfa.event.newText.length == 2) xfa.host.setFocus(validate);</script></event>");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(&template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    let generation = a.session().get(DocId(id)).unwrap().edit_generation();
    let result = call(&mut a, "form_change_preview", json!({"doc":id,"field":"month","text":"1"}));
    assert_eq!(result["requests"], json!([]));
    let result = call(&mut a, "form_change_preview", json!({"doc":id,"field":"month","text":"12"}));
    assert_eq!(result["requests"], json!([{"focus":"validate"}]));
    assert_eq!(a.session().get(DocId(id)).unwrap().edit_generation(), generation);
    assert!(a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name == "month").unwrap().value.iter().all(String::is_empty));
}

#[test]
fn initialize_populates_dependent_choices_from_xml_attributes() {
    let template = r#"<xdp xmlns="http://ns.adobe.com/xdp/"><template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><subform name="form" layout="tb">
<field name="region" w="180pt" h="24pt"><ui><choiceList/></ui><items><text>Old label</text></items><items save="1"><text>old</text></items>
<event activity="initialize"><script contentType="application/x-javascript">
var old = this.rawValue;
this.clearItems();
var entries = xfa.resolveNodes('xfa.datasets.choices.region[*]');
for (var i = 0; i &lt; entries.length; i++) { var entry = entries.item(i); this.addItem(entry.value, entry.code.value); }
if (old == null) this.selectedIndex = 0; else this.rawValue = old;
</script></event></field></subform></template>
<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data/><choices><region code="a">Alpha</region><region code="b">Beta</region></choices></xfa:datasets></xdp>"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let opened = call(&mut a, "doc_open", json!({"path":"form.pdf"}));
    assert!(opened["errors"].as_array().is_none_or(Vec::is_empty), "{opened}");
    let id = opened["doc"].as_u64().unwrap();
    let doc = a.session().get(DocId(id)).unwrap();
    let f = doc.form.iter().find(|f| f.name == "region").unwrap();
    assert_eq!(f.options, [("a".into(), "Alpha".into()), ("b".into(), "Beta".into())]);
    assert_eq!(f.value, ["a"]);
    call(&mut a, "form_fill", json!({"doc":id,"values":{"region":"b"}}));
    call(&mut a, "doc_save", json!({"doc":id,"path":"saved.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"saved.pdf"}))["doc"].as_u64().unwrap();
    assert_eq!(a.session().get(DocId(reopened)).unwrap().form.iter().find(|f| f.name == "region").unwrap().value, ["b"]);
}

#[test]
fn remerge_discards_saved_instance_state_and_undo_restores_it() {
    let template = r#"<xdp xmlns="http://ns.adobe.com/xdp/"><template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><subform name="form" layout="tb">
<subform name="details" presence="hidden"><field name="value" w="100pt" h="20pt"><ui><textEdit/></ui></field></subform>
<field name="clear" w="100pt" h="20pt"><ui><button/></ui><event activity="click"><script contentType="application/x-javascript">xfa.datasets.data.nodes.remove(xfa.datasets.data.form); xfa.form.remerge();</script></event></field>
</subform></template><form xmlns="http://www.xfa.org/schema/xfa-form/2.8/"><subform name="form"><subform name="details" presence="visible"/></subform></form>
<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><details><value>Example</value></details></form></xfa:data><external>Keep this</external></xfa:datasets></xdp>"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    assert!(a.session().get(DocId(id)).unwrap().form.iter().any(|f| f.name == "value"));
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"clear","script":""}));
    assert!(result["error"].is_null(), "{result}");
    assert!(!a.session().get(DocId(id)).unwrap().form.iter().any(|f| f.name == "value"));
    call(&mut a, "doc_save", json!({"doc":id,"path":"cleared.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"cleared.pdf"}))["doc"].as_u64().unwrap();
    assert!(!a.session().get(DocId(reopened)).unwrap().form.iter().any(|f| f.name == "value"));
    call(&mut a, "edit_undo", json!({"doc":id}));
    assert_eq!(a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name == "value").unwrap().value, ["Example"]);
    call(&mut a, "edit_redo", json!({"doc":id}));
    assert!(!a.session().get(DocId(id)).unwrap().form.iter().any(|f| f.name == "value"));
}

#[test]
fn nested_events_resolve_the_targets_siblings_without_changing_the_callers_scope() {
    let template = r#"<template><subform name="form" layout="tb">
<field name="No" w="100pt" h="20pt"><ui><textEdit/></ui><value><text>caller</text></value></field>
<subform name="section"><field name="No" w="100pt" h="20pt"><ui><textEdit/></ui></field>
<field name="target" w="100pt" h="20pt"><ui><textEdit/></ui><validate><script contentType="application/x-javascript">No.rawValue = 'nested'; if (xfa.resolveNode('No').rawValue != 'nested') throw Error('wrong relative scope'); this.rawValue = 'target';</script></validate></field></subform>
<field name="check" w="100pt" h="20pt"><ui><button/></ui><event activity="click"><script contentType="application/x-javascript">section.target.execEvent('validate'); xfa.host.messageBox(No.rawValue);</script></event></field>
</subform></template>"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"check","script":""}));
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(result["alerts"], json!(["caller"]));
    let doc = a.session().get(DocId(id)).unwrap();
    assert_eq!(doc.form.iter().find(|f| f.name == "No_2").unwrap().value, ["nested"]);
    assert_eq!(doc.form.iter().find(|f| f.name == "target").unwrap().value, ["target"]);
}

#[test]
fn saved_paper_barcodes_render_and_manifests_regenerate_them_after_edits() {
    let template = r#"<xdp><template><subform name="form" layout="tb">
<variables><manifest name="collection"><ref>form.name.dataNode</ref><ref>form.month.dataNode</ref></manifest></variables>
<field name="name" w="150pt" h="20pt"><bind match="dataRef" ref="$record.person"/><ui><textEdit/></ui><value><text>Example</text></value></field>
<field name="month" w="30pt" h="20pt"><ui><textEdit/></ui><bind><picture>num{99}</picture></bind><format><picture>num{99}</picture></format></field>
<field name="barcode" w="270pt" h="144pt"><bind match="none"/><ui><barcode type="pdf417" errorCorrectionLevel="5" moduleWidth="1pt" moduleHeight="2pt"/></ui></field>
<field name="check" w="100pt" h="20pt"><ui><button/></ui><event activity="click"><script contentType="application/x-javascript">
if (xfa.resolveNode('form.name').dataNode.value !== 'Changed') throw new Error('Wrong bound answer');
if (xfa.resolveNode('form.month').rawValue !== '5') throw new Error('Wrong canonical month');
xfa.resolveNode('form.name').rawValue = 'Updated';
var parts = ['TEST']; for (var i = 0; i &lt; collection.nodes.length; i++) parts.push(xfa.resolveNode(collection.nodes.item(i).value).value);
barcode.rawValue = parts.join('|'); xfa.host.messageBox(barcode.rawValue);
</script></event></field>
<field name="read" w="100pt" h="20pt"><ui><button/></ui><event activity="click"><script contentType="application/x-javascript">xfa.host.messageBox(barcode.rawValue);</script></event></field>
</subform></template><form xmlns="http://www.xfa.org/schema/xfa-form/2.8/"><subform name="form"><field name="barcode"><value><text>Saved barcode</text></value></field></subform></form></xdp>"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"read","script":""}));
    assert_eq!(result["alerts"], json!(["Saved barcode"]));
    let before = a.call("page_render", &json!({"doc":id,"page":1,"dpi":72})).unwrap();
    assert!(before.iter().any(|c| matches!(c, Content::Png { .. })));
    call(&mut a, "form_fill", json!({"doc":id,"values":{"name":"Changed","month":"05"}}));
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"check","script":""}));
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(result["alerts"], json!(["TEST|Updated|05"]));
    call(&mut a, "doc_save", json!({"doc":id,"path":"saved.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"saved.pdf"}))["doc"].as_u64().unwrap();
    let result = call(&mut a, "js_run", json!({"doc":reopened,"field":"read","script":""}));
    assert_eq!(result["alerts"], json!(["TEST|Updated|05"]));
    // Decode the actual PDF page after editing, saving and reopening, rather than
    // merely checking that the encoder or a content stream contains some bars.
    use rxing::{BinaryBitmap, Luma8LuminanceSource, Reader, common::HybridBinarizer, pdf417::PDF417Reader};
    let rendered = a.call("page_render", &json!({"doc":reopened,"page":1,"dpi":144})).unwrap();
    let png = rendered.iter().find_map(|c| if let Content::Png { data, .. } = c { Some(data) } else { None }).unwrap();
    let pixels = image::load_from_memory(png).unwrap().to_luma8();
    let (width, height) = pixels.dimensions();
    let source = Luma8LuminanceSource::new(pixels.into_raw(), width, height).unwrap();
    let decoded = PDF417Reader.decode(&mut BinaryBitmap::new(HybridBinarizer::new(source))).unwrap();
    assert_eq!(decoded.getText(), "TEST|Updated|05");
    call(&mut a, "edit_undo", json!({"doc":id}));
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"read","script":""}));
    assert_eq!(result["alerts"], json!(["Saved barcode"]));
}

#[test]
fn radio_child_assignments_update_the_group_without_creating_structured_data() {
    let template = r#"<xdp><template><subform name="form" layout="tb">
<exclGroup name="choice"><field name="No" w="50pt" h="20pt"><ui><checkButton/></ui><items><text>0</text><text/></items></field><field name="Yes" x="60pt" w="50pt" h="20pt"><ui><checkButton/></ui><items><text>1</text><text/></items></field></exclGroup>
<field name="check" w="100pt" h="20pt"><ui><button/></ui><event activity="click"><script contentType="application/x-javascript">choice.Yes.rawValue = '1'; choice.No.rawValue = null; xfa.host.messageBox(String(choice.rawValue) + '|' + choice.Yes.rawValue + '|' + choice.No.rawValue);</script></event></field>
</subform></template><xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><choice>0</choice></form></xfa:data></xfa:datasets></xdp>"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"check","script":""}));
    assert!(result["error"].is_null(), "{result}");
    assert_eq!(result["alerts"], json!(["1|1|null"]));
    assert_eq!(a.session().get(DocId(id)).unwrap().form.iter().find(|f| f.name == "choice").unwrap().value, ["1"]);
    call(&mut a, "doc_save", json!({"doc":id,"path":"saved.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"saved.pdf"}))["doc"].as_u64().unwrap();
    assert_eq!(a.session().get(DocId(reopened)).unwrap().form.iter().find(|f| f.name == "choice").unwrap().value, ["1"]);
}

#[test]
fn presave_encodes_current_data_and_failed_scripts_leave_the_file_untouched() {
    let template = r#"<xdp><template><subform name="form" layout="tb">
<event activity="preSave"><script contentType="application/x-javascript">
barcode.rawValue = name.rawValue;
if (name.rawValue == 'fail') throw Error('cannot encode');
if (xfa.data.saveXML().indexOf('Changed &amp;amp; escaped') == -1) throw Error('stale data');
</script></event>
<field name="name" w="150pt" h="20pt"><ui><textEdit/></ui></field>
<field name="barcode" w="270pt" h="144pt"><bind match="none"/><ui><barcode type="pdf417"/></ui></field>
<field name="read" w="100pt" h="20pt"><ui><button/></ui><event activity="click"><script contentType="application/x-javascript">xfa.host.messageBox(barcode.rawValue);</script></event></field>
</subform></template><xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><name>Before</name></form></xfa:data></xfa:datasets></xdp>"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(template)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({"path":"form.pdf"}))["doc"].as_u64().unwrap();
    call(&mut a, "form_fill", json!({"doc":id,"values":{"name":"Changed & escaped"}}));
    call(&mut a, "doc_save", json!({"doc":id,"path":"saved.pdf"}));
    let reopened = call(&mut a, "doc_open", json!({"path":"saved.pdf"}))["doc"].as_u64().unwrap();
    let result = call(&mut a, "js_run", json!({"doc":reopened,"field":"read","script":""}));
    assert_eq!(result["alerts"], json!(["Changed & escaped"]));
    let saved = std::fs::read(dir.path().join("saved.pdf")).unwrap();
    call(&mut a, "form_fill", json!({"doc":id,"values":{"name":"fail"}}));
    let generation = a.session().get(DocId(id)).unwrap().edit_generation();
    assert!(a.call("doc_save", &json!({"doc":id,"path":"saved.pdf"})).is_err());
    assert_eq!(a.session().get(DocId(id)).unwrap().edit_generation(), generation);
    assert_eq!(std::fs::read(dir.path().join("saved.pdf")).unwrap(), saved);
    let result = call(&mut a, "js_run", json!({"doc":id,"field":"read","script":""}));
    assert_eq!(result["alerts"], json!(["Changed & escaped"]));
}
