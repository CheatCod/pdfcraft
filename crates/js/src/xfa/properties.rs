//! Script-visible XFA property nodes. Changes are recorded as bounded effects, just like
//! field values; the engine persists and renders supported properties.
use super::*;

const PATH: &str = "__xfaProperty";

pub(super) fn object(ctx: &mut Context, idx: usize, path: &str) -> JsResult<JsValue> {
    if path.len() > 512 {
        return Err(error("XFA property path is too long"));
    }
    let target = ObjectInitializer::new(ctx)
        .property(JsString::from(IDX), JsValue::from(idx as f64), Attribute::READONLY)
        .property(JsString::from(PATH), s(path), Attribute::READONLY)
        .build();
    Ok(JsProxy::builder(target).get(get).set(set).build(ctx)?.into())
}

fn location(value: &JsValue, ctx: &mut Context) -> JsResult<(usize, String)> {
    let idx = node_index(value, ctx)?;
    let o = value.as_object().ok_or_else(|| error("expected an XFA property"))?;
    Ok((idx, text(&o.get(JsString::from(PATH), ctx)?, ctx)?))
}

pub(super) fn value(ctx: &Context, idx: usize, path: &str) -> String {
    host(ctx).ok().and_then(|h| h.borrow().nodes.get(idx).and_then(|n| n.properties.get(path).cloned())).unwrap_or_else(|| {
        match path.rsplit('.').next().unwrap_or("") {
            "presence" => "visible",
            "nullTest" => "disabled",
            "thickness" => "0.5pt",
            "stroke" => "solid",
            "open" => "userControl",
            _ => "",
        }
        .to_string()
    })
}

pub(super) fn assign(ctx: &Context, idx: usize, path: String, value: String) -> JsResult<()> {
    if path.len() > 512 {
        return Err(error("XFA property path is too long"));
    }
    let (value, _) = clip_value(value);
    let mut h = host(ctx)?.borrow_mut();
    let Some(n) = h.nodes.get_mut(idx) else { return Ok(()) };
    if n.properties.len() >= 512 && !n.properties.contains_key(&path) {
        return Err(error("XFA object has more than 512 properties"));
    }
    if n.properties.get(&path) == Some(&value) {
        return Ok(());
    }
    n.properties.insert(path.clone(), value.clone());
    let som = n.som.clone();
    h.push(XfaEffect::SetProperty { som, path, value });
    Ok(())
}

fn get(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = arg(args, 0);
    let (idx, path) = location(&target, ctx)?;
    if arg(args, 1).as_symbol().is_some() {
        return Ok(JsValue::undefined());
    }
    let key = text(&arg(args, 1), ctx)?;
    match key.as_str() {
        IDX => return Ok(JsValue::from(idx as f64)),
        PATH => return Ok(s(&path)),
        "className" => return Ok(s(path.rsplit('.').next().unwrap_or("").split('[').next().unwrap_or(""))),
        "parent" => {
            return match path.rsplit_once('.') {
                Some((p, _)) => object(ctx, idx, p),
                None => node_object(ctx, idx),
            };
        }
        "oneOfChild" => {
            let child = if path == "ui" { value(ctx, idx, "uiType") } else { "solid".into() };
            return object(ctx, idx, &format!("{path}.{child}"));
        }
        "getElement" | "resolveNode" | "getAttribute" | "isPropertySpecified" => {
            let method = ObjectInitializer::new(ctx)
                .property(js_string!("target"), target, Attribute::READONLY)
                .property(js_string!("method"), s(&key), Attribute::READONLY)
                .build();
            return Ok(function(ctx, NativeFunction::from_copy_closure_with_captures(call, method)).into());
        }
        "nodes" => {
            let method = ObjectInitializer::new(ctx)
                .property(js_string!("target"), target, Attribute::READONLY)
                .property(js_string!("method"), s("remove"), Attribute::READONLY)
                .build();
            let remove = function(ctx, NativeFunction::from_copy_closure_with_captures(call, method));
            return Ok(ObjectInitializer::new(ctx)
                .property(js_string!("remove"), remove, Attribute::READONLY)
                .function(NativeFunction::from_fn_ptr(noop), js_string!("append"), 1)
                .build()
                .into());
        }
        _ => {}
    }
    let key = if key == "edge[0]" { "edge" } else { &key };
    let full = format!("{path}.{key}");
    if key == "value" && path == "caption" {
        return object(ctx, idx, &full);
    }
    if matches!(
        key,
        "value" | "presence" | "thickness" | "stroke" | "join" | "radius" | "inverted" | "open" | "nullTest" | "scriptTest" | "formatTest"
    ) {
        return Ok(s(&value(ctx, idx, &full)));
    }
    object(ctx, idx, &full)
}

fn set(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (idx, path) = location(&arg(args, 0), ctx)?;
    if arg(args, 1).as_symbol().is_some() {
        return Ok(JsValue::from(false));
    }
    let key = text(&arg(args, 1), ctx)?;
    let text = text(&arg(args, 2), ctx)?;
    assign(ctx, idx, format!("{path}.{key}"), text)?;
    Ok(JsValue::from(true))
}

fn call(_: &JsValue, args: &[JsValue], capture: &JsObject, ctx: &mut Context) -> JsResult<JsValue> {
    let target = capture.get(js_string!("target"), ctx)?;
    let (idx, path) = location(&target, ctx)?;
    let method = text(&capture.get(js_string!("method"), ctx)?, ctx)?;
    let name = if method == "remove" { String::new() } else { text(&arg(args, 0), ctx)? };
    match method.as_str() {
        "getAttribute" => Ok(s(&value(ctx, idx, &format!("{path}.{name}")))),
        "isPropertySpecified" => {
            let prefix = format!("{path}.{name}");
            Ok(JsValue::from(host(ctx)?.borrow().nodes.get(idx).is_some_and(|n| n.properties.keys().any(|p| p.starts_with(&prefix)))))
        }
        "remove" => {
            let (_, removed) = location(&arg(args, 0), ctx)?;
            // Removing a generated border restores its template appearance.
            assign(ctx, idx, format!("{removed}.presence"), "invisible".into())?;
            Ok(JsValue::undefined())
        }
        _ => {
            let name = name.trim_start_matches('#');
            let index = if method == "getElement" { arg(args, 1).to_number(ctx).unwrap_or(0.0) } else { 0.0 };
            if !index.is_finite() || !(0.0..=255.0).contains(&index) {
                return Err(error("XFA property index must be between 0 and 255"));
            }
            let child = if index == 0.0 { name.to_string() } else { format!("{name}[{}]", index as usize) };
            object(ctx, idx, &format!("{path}.{child}"))
        }
    }
}
