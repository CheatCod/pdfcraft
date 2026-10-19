//! Bounded XML data sources in the XFA data model. No network access or external entities.
use super::*;

pub(super) fn parse(xml: &str) -> Result<XfaNode, String> {
    if xml.len() > 8 * 1024 * 1024 || xml_nesting_depth(xml) > MAX_DEPTH {
        return Err("XFA data XML exceeds the size or nesting limit".into());
    }
    let parsed = roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 100_000 })
        .map_err(|e| e.to_string())?;
    let root = parsed.descendants().find(|n| n.is_element() && n.tag_name().name() == "datasets").unwrap_or_else(|| parsed.root_element());
    fn node(n: roxmltree::Node, parent: &str, index: usize) -> XfaNode {
        let name = n.tag_name().name().to_string();
        let som = if parent.is_empty() && name == "datasets" {
            "xfa.datasets".into()
        } else if parent.is_empty() {
            format!("{name}[{index}]")
        } else {
            format!("{parent}.{name}[{index}]")
        };
        let mut counts = HashMap::<String, usize>::new();
        let children = n
            .children()
            .filter(|c| c.is_element())
            .map(|c| {
                let i = counts.entry(c.tag_name().name().to_string()).or_default();
                let child = node(c, &som, *i);
                *i += 1;
                child
            })
            .collect::<Vec<_>>();
        let value = n.children().filter(|n| n.is_text()).filter_map(|n| n.text()).collect::<String>();
        XfaNode {
            name,
            som,
            index,
            kind: Some(if children.is_empty() { XfaKind::DataValue } else { XfaKind::DataGroup }),
            value,
            properties: n.attributes().map(|a| (format!("@{}.value", a.name()), a.value().to_string())).collect(),
            children,
            ..Default::default()
        }
    }
    Ok(node(root, "", 0))
}

pub(super) fn method(ctx: &mut Context, idx: usize, name: &str) -> JsResult<JsValue> {
    let capture = ObjectInitializer::new(ctx)
        .property(js_string!("index"), idx as f64, Attribute::READONLY)
        .property(js_string!("method"), s(name), Attribute::READONLY)
        .build();
    Ok(function(ctx, NativeFunction::from_copy_closure_with_captures(call, capture)).into())
}

pub(super) fn children(ctx: &mut Context, idx: usize) -> JsResult<JsValue> {
    let kids = host(ctx)?.borrow().nodes.get(idx).map(|n| n.children.clone()).unwrap_or_default();
    let list = list_object(ctx, &kids)?.as_object().ok_or_else(|| error("invalid data list"))?;
    for op in ["append", "remove"] {
        let f = method(ctx, idx, op)?;
        list.set(JsString::from(op), f, true, ctx)?;
    }
    Ok(list.into())
}

fn call(_: &JsValue, args: &[JsValue], capture: &JsObject, ctx: &mut Context) -> JsResult<JsValue> {
    let idx = capture.get(js_string!("index"), ctx)?.to_number(ctx)? as usize;
    let op = text(&capture.get(js_string!("method"), ctx)?, ctx)?;
    match op.as_str() {
        "saveXML" => Ok(s(&serialize(&host(ctx)?.borrow(), idx).map_err(|e| error(&e))?)),
        "createNode" => {
            let class = text(&arg(args, 0), ctx)?;
            let name = text(&arg(args, 1), ctx)?;
            if !is_identifier(&name) || name.contains('$') || name.len() > 256 {
                return Err(error("invalid XFA data node name"));
            }
            let node = XfaNode { name, kind: Some(if class == "dataGroup" { XfaKind::DataGroup } else { XfaKind::DataValue }), ..Default::default() };
            let i = flatten(&node, None, &mut host(ctx)?.borrow_mut().nodes, 0).ok_or_else(|| error("too many XFA data nodes"))?;
            node_object(ctx, i)
        }
        "append" => {
            let child = node_index(&arg(args, 0), ctx)?;
            let mut h = host(ctx)?.borrow_mut();
            if child == idx || h.nodes.get(child).is_none_or(|n| n.parent.is_some()) {
                return Err(error("only an unattached data node can be appended"));
            }
            // An unattached root can still be an ancestor of the destination.
            // Reject that cycle before changing either node.
            let mut ancestor = Some(idx);
            for _ in 0..=MAX_DEPTH {
                let Some(at) = ancestor else { break };
                if at == child {
                    return Err(error("an XFA data node cannot contain itself"));
                }
                ancestor = h.nodes.get(at).and_then(|n| n.parent);
            }
            if ancestor.is_some() {
                return Err(error("XFA data nesting exceeds the depth limit"));
            }
            if let Some(n) = h.nodes.get_mut(child) {
                n.parent = Some(idx);
            }
            if let Some(n) = h.nodes.get_mut(idx) {
                n.children.push(child);
            }
            renumber(&mut h, child, 0);
            Ok(JsValue::undefined())
        }
        "remove" => remove_data(&JsValue::undefined(), args, ctx),
        "loadXML" => {
            let xml = text(&arg(args, 0), ctx)?;
            let parsed = parse(&xml).map_err(|e| error(&e))?;
            let ignore_root = arg(args, 1).to_boolean();
            let overwrite = arg(args, 2).to_boolean();
            let mut h = host(ctx)?.borrow_mut();
            let som = h.nodes.get(idx).map(|n| n.som.clone()).unwrap_or_default();
            if !som.starts_with("xfa.datasets.") {
                return Err(error("loadXML requires an attached data node"));
            }
            let children = if ignore_root { parsed.children } else { vec![parsed] };
            if overwrite && let Some(n) = h.nodes.get_mut(idx) {
                n.children.clear();
                n.value.clear();
            }
            for child in children {
                let ci = flatten(&child, Some(idx), &mut h.nodes, 0).ok_or_else(|| error("too many XFA data nodes"))?;
                if let Some(n) = h.nodes.get_mut(idx) {
                    n.children.push(ci);
                }
                renumber(&mut h, ci, 0);
            }
            h.push(XfaEffect::LoadDataXml { som, xml, ignore_root, overwrite });
            Ok(JsValue::undefined())
        }
        _ => Err(error("unsupported XFA data operation")),
    }
}

/// Serialize data nodes only, within the same size/depth limits as loadXML.
fn serialize(h: &XHost, idx: usize) -> Result<String, String> {
    const LIMIT: usize = 8 * 1024 * 1024;
    fn append(out: &mut String, s: &str) -> Result<(), String> {
        if out.len().saturating_add(s.len()) > LIMIT {
            return Err("XFA data XML exceeds 8 MiB".into());
        }
        out.push_str(s);
        Ok(())
    }
    fn escaped(out: &mut String, s: &str) -> Result<(), String> {
        for c in s.chars() {
            if (c < ' ' && !matches!(c, '\t' | '\n' | '\r')) || matches!(c, '\u{fffe}' | '\u{ffff}') {
                return Err("XFA data contains a character XML cannot represent".into());
            }
            let mut b = [0; 4];
            append(
                out,
                match c {
                    '&' => "&amp;",
                    '<' => "&lt;",
                    '>' => "&gt;",
                    '"' => "&quot;",
                    '\r' => "&#13;",
                    _ => c.encode_utf8(&mut b),
                },
            )?;
        }
        Ok(())
    }
    fn node(h: &XHost, idx: usize, out: &mut String, depth: usize, budget: &mut usize) -> Result<(), String> {
        if depth > MAX_DEPTH || *budget == 0 {
            return Err("XFA data XML exceeds its node or nesting limit".into());
        }
        *budget -= 1;
        let n = h.nodes.get(idx).ok_or("invalid XFA data node")?;
        if n.gone {
            return Ok(());
        }
        append(out, "<")?;
        append(out, &n.name)?;
        for (key, value) in &n.properties {
            if let Some(name) = key.strip_prefix('@').and_then(|s| s.strip_suffix(".value")) {
                append(out, " ")?;
                append(out, name)?;
                append(out, "=\"")?;
                escaped(out, value)?;
                append(out, "\"")?;
            }
        }
        append(out, ">")?;
        escaped(out, &n.value)?;
        for &child in &n.children {
            node(h, child, out, depth + 1, budget)?;
        }
        append(out, "</")?;
        append(out, &n.name)?;
        append(out, ">")
    }
    let mut out = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n".to_string();
    node(h, idx, &mut out, 0, &mut 100_000)?;
    Ok(out)
}

fn xml_nesting_depth(text: &str) -> usize {
    fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
        (from..hay.len()).find(|&p| hay.get(p..).is_some_and(|tail| tail.starts_with(needle)))
    }
    let b = text.as_bytes();
    let (mut i, mut depth, mut max) = (0usize, 0usize, 0usize);
    while i < b.len() {
        if b.get(i) != Some(&b'<') {
            i += 1;
        } else if b.get(i..).is_some_and(|tail| tail.starts_with(b"<!--")) {
            i = find(b, i + 4, b"-->").map_or(b.len(), |p| p + 3);
        } else if b.get(i..).is_some_and(|tail| tail.starts_with(b"<![CDATA[")) {
            i = find(b, i + 9, b"]]>").map_or(b.len(), |p| p + 3);
        } else if b.get(i..).is_some_and(|tail| tail.starts_with(b"<?")) {
            i = find(b, i + 2, b"?>").map_or(b.len(), |p| p + 2);
        } else if b.get(i..).is_some_and(|tail| tail.starts_with(b"<!")) {
            i = find(b, i + 2, b">").map_or(b.len(), |p| p + 1);
        } else if b.get(i..).is_some_and(|tail| tail.starts_with(b"</")) {
            depth = depth.saturating_sub(1);
            i = find(b, i + 2, b">").map_or(b.len(), |p| p + 1);
        } else {
            depth += 1;
            max = max.max(depth);
            // Skip the rest of the start tag, over any quoted attribute value, and remember its last
            // significant byte: a tag that ends in `/` closes itself, so it is one level deep while it
            // is read rather than a level left open.
            i += 1;
            let mut quote = 0u8;
            let mut last = 0u8;
            while i < b.len() {
                let Some(&c) = b.get(i) else { break };
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'>' {
                    i += 1;
                    break;
                } else if !c.is_ascii_whitespace() {
                    last = c;
                }
                i += 1;
            }
            if last == b'/' {
                depth -= 1;
            }
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_sources_load_and_resolve_without_aliasing_form_nodes() {
        let doc = XfaDoc {
            datasets_xml: "<datasets><data><application><value>saved</value></application></data><lists><old/></lists></datasets>".into(),
            ..Default::default()
        };
        let root = XfaNode { name: "application".into(), som: "application[0]".into(), kind: Some(XfaKind::Subform), ..Default::default() };
        let out = run_xfa(
            "var list = xfa.datasets.resolveNode('lists'); list.loadXML('<choices><entry>one</entry><entry>two</entry></choices>', false, true); xfa.host.messageBox(xfa.resolveNodes('xfa.datasets.lists.choices.entry[*]').item(1).value); xfa.datasets.data.nodes.remove(xfa.resolveNode('xfa.datasets.data.application'));",
            &XfaEvent { target: "application[0]".into(), ..Default::default() },
            &doc,
            &root,
            Limits::default(),
        );
        assert!(out.error.is_none(), "{out:?}");
        assert!(out.effects.contains(&XfaEffect::MessageBox("two".into())));
        assert!(out.effects.contains(&XfaEffect::RemoveData("xfa.datasets.data[0].application[0]".into())));
    }

    #[test]
    fn appending_an_unattached_ancestor_rejects_the_cycle() {
        let doc = XfaDoc { datasets_xml: "<datasets><data/></datasets>".into(), ..Default::default() };
        let root = XfaNode { name: "application".into(), som: "application[0]".into(), ..Default::default() };
        let out = run_xfa(
            "var a = xfa.datasets.createNode('dataGroup', 'a'); var b = xfa.datasets.createNode('dataGroup', 'b'); a.nodes.append(b); b.nodes.append(a);",
            &XfaEvent { target: "application[0]".into(), ..Default::default() },
            &doc,
            &root,
            Limits::default(),
        );
        assert!(out.error.as_deref().is_some_and(|e| e.contains("cannot contain itself")), "{out:?}");
    }

    #[test]
    fn hostile_xml_is_rejected_before_recursive_parsing() {
        assert!(parse(&format!("{}{}", "<a>".repeat(2000), "</a>".repeat(2000))).is_err());
        assert!(parse("<!DOCTYPE a [<!ENTITY x SYSTEM 'file:///private'>]><a>&x;</a>").is_err());
    }
}
