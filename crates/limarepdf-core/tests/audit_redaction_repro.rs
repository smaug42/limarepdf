use limarepdf_core::Pdf;
use lopdf::{dictionary, Dictionary, Object, Stream};

#[test]
fn audit_named_properties_retain_redacted_actual_text() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding"});
    let properties = p.doc.add_object(dictionary! {"ActualText"=>Object::string_literal("ZebraSecret"), "Alt"=>Object::string_literal("ZebraSecret")});
    let content = p.doc.add_object(Stream::new(Dictionary::new(), b"/Span /P1 BDC BT /F1 20 Tf 30 100 Td (ZebraSecret) Tj ET EMC".to_vec()));
    let page = p.page_ids()[0];
    let pd = p.doc.get_dictionary_mut(page).unwrap();
    pd.set("Resources", dictionary! {"Font"=>dictionary!{"F1"=>font}, "Properties"=>dictionary!{"P1"=>properties}});
    pd.set("Contents", content);
    let marks = p.find_redaction_marks("ZebraSecret").unwrap();
    assert!(!marks.is_empty());
    let report = p.redact(&marks).unwrap();
    assert!(report.left.is_empty());
    assert!(report.elsewhere.is_empty());
    let saved = p.to_bytes().unwrap();
    let reopened = Pdf::from_bytes(&saved, None).unwrap();
    let res = reopened.doc.get_dictionary(page).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
    let prop_ref = res.get(b"Properties").unwrap().as_dict().unwrap().get(b"P1").unwrap().as_reference().unwrap();
    let retained = reopened.doc.get_dictionary(prop_ref).unwrap().get(b"ActualText").unwrap().as_str().unwrap();
    assert_eq!(retained, b"ZebraSecret");
    eprintln!("CONFIRMED: redaction reported zero leftovers and no elsewhere warning; saved Resources/Properties/P1/ActualText still contains {}", String::from_utf8_lossy(retained));
}

#[test]
fn audit_indirect_next_array_retains_javascript() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let script = p.doc.add_object(dictionary! {"S"=>"JavaScript", "JS"=>Object::string_literal("app.alert('AUDIT_SECRET_SCRIPT')")});
    let next = p.doc.add_object(vec![Object::Reference(script)]);
    let action = p.doc.add_object(dictionary! {"S"=>"GoTo", "D"=>vec![Object::Reference(p.page_ids()[0]), Object::Name(b"Fit".to_vec())], "Next"=>next});
    p.doc.catalog_mut().unwrap().set("OpenAction", action);
    let rep = p.remove_hidden(&[limarepdf_core::Hidden::Scripts]).unwrap();
    let saved = p.to_bytes().unwrap();
    let reopened = Pdf::from_bytes(&saved, None).unwrap();
    assert!(reopened.doc.objects.get(&script).is_some());
    assert!(reopened.doc.get_dictionary(action).unwrap().has(b"Next"));
    eprintln!("CONFIRMED: script cleanup count {}; saved OpenAction/Next -> indirect array -> JavaScript remains reachable", rep.count(limarepdf_core::Hidden::Scripts));
}

#[test]
fn audit_ve_hidden_content_remains() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding"});
    let off = p.doc.add_object(dictionary! {"Type"=>"OCG", "Name"=>Object::string_literal("Off")});
    let on = p.doc.add_object(dictionary! {"Type"=>"OCG", "Name"=>Object::string_literal("On")});
    let membership = p.doc.add_object(dictionary! {"Type"=>"OCMD", "OCGs"=>vec![Object::Reference(on), Object::Reference(off)], "P"=>"AnyOn", "VE"=>vec![Object::Name(b"And".to_vec()), Object::Reference(on), Object::Reference(off)]});
    let content = p.doc.add_object(Stream::new(Dictionary::new(), b"/OC /P1 BDC BT /F1 20 Tf 30 100 Td (VEHiddenSecret) Tj ET EMC".to_vec()));
    let page = p.page_ids()[0];
    let pd = p.doc.get_dictionary_mut(page).unwrap();
    pd.set("Resources", dictionary! {"Font"=>dictionary!{"F1"=>font}, "Properties"=>dictionary!{"P1"=>membership}});
    pd.set("Contents", content);
    p.doc.catalog_mut().unwrap().set("OCProperties", dictionary! {"OCGs"=>vec![Object::Reference(off),Object::Reference(on)], "D"=>dictionary!{"OFF"=>vec![Object::Reference(off)]}});
    p.save("target/audit-ve-before.pdf").unwrap();
    let rep = p.remove_hidden(&[limarepdf_core::Hidden::HiddenLayers]).unwrap();
    p.save("target/audit-ve-after.pdf").unwrap();
    assert!(p.page_text(0).contains("VEHiddenSecret"));
    eprintln!("CONFIRMED: hidden-layer cleanup count {}; VEHiddenSecret remains in saved page content", rep.count(limarepdf_core::Hidden::HiddenLayers));
}

#[test]
fn audit_form_inheriting_resources_keeps_unredacted_copy() {
    let mut p = Pdf::new();
    p.insert_blank(0, 300.0, 200.0).unwrap();
    let font = p.doc.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding"});
    let form = p.doc.add_object(Stream::new(dictionary! {"Type"=>"XObject", "Subtype"=>"Form", "BBox"=>vec![0.into(),0.into(),300.into(),200.into()]}, b"BT /F1 20 Tf 30 100 Td (FormSecretZebra) Tj ET".to_vec()));
    let content = p.doc.add_object(Stream::new(Dictionary::new(), b"q /Fm1 Do Q".to_vec()));
    let page = p.page_ids()[0];
    let pd = p.doc.get_dictionary_mut(page).unwrap();
    pd.set("Resources", dictionary! {"Font"=>dictionary!{"F1"=>font}, "XObject"=>dictionary!{"Fm1"=>form}});
    pd.set("Contents", content);
    let marks = p.find_redaction_marks("FormSecretZebra").unwrap();
    assert!(!marks.is_empty());
    let report = p.redact(&marks).unwrap();
    assert!(report.left.is_empty());
    assert!(report.elsewhere.is_empty());
    let saved = p.to_bytes().unwrap();
    let reopened = Pdf::from_bytes(&saved, None).unwrap();
    assert!(!reopened.page_text(0).contains("FormSecretZebra"));
    let retained = reopened.doc.get_object(form).unwrap().as_stream().unwrap().get_plain_content().unwrap();
    assert!(String::from_utf8_lossy(&retained).contains("FormSecretZebra"));
    eprintln!("CONFIRMED: redacted form copied inherited resource dictionary containing reference to its own unredacted original; original secret stream remains in saved file");
}
