//! Bounded choice-list mutations used by dependent dropdowns.
use super::*;

pub(super) fn items(ctx: &Context, idx: usize) -> Vec<(String, String)> {
    let json = properties::value(ctx, idx, "$items");
    if json.len() > 65_536 {
        return Vec::new();
    }
    serde_json::from_str::<Vec<(String, String)>>(&json).ok().filter(|v| v.len() <= 1000).unwrap_or_default()
}

pub(super) fn method(ctx: &mut Context, idx: usize, name: &str) -> JsResult<JsValue> {
    let capture = ObjectInitializer::new(ctx)
        .property(js_string!("index"), idx as f64, Attribute::READONLY)
        .property(js_string!("method"), s(name), Attribute::READONLY)
        .build();
    Ok(function(ctx, NativeFunction::from_copy_closure_with_captures(call, capture)).into())
}

fn call(_: &JsValue, args: &[JsValue], capture: &JsObject, ctx: &mut Context) -> JsResult<JsValue> {
    let idx = capture.get(js_string!("index"), ctx)?.to_number(ctx)? as usize;
    let op = text(&capture.get(js_string!("method"), ctx)?, ctx)?;
    let mut list = items(ctx, idx);
    match op.as_str() {
        "clearItems" => {
            list.clear();
            host(ctx)?.borrow_mut().set_value(idx, String::new());
        }
        "addItem" => {
            if list.len() >= 1000 {
                return Err(error("an XFA choice list may contain at most 1000 items"));
            }
            let label = text(&arg(args, 0), ctx)?;
            let saved = if arg(args, 1).is_undefined() { label.clone() } else { text(&arg(args, 1), ctx)? };
            list.push((saved, label));
        }
        "boundItem" => {
            let label = text(&arg(args, 0), ctx)?;
            return Ok(list.iter().find(|(_, shown)| shown == &label).map_or(JsValue::null(), |(saved, _)| s(saved)));
        }
        _ => {
            let i = arg(args, 0).to_number(ctx)?;
            if !i.is_finite() || i < 0.0 || i.fract() != 0.0 || i >= list.len() as f64 {
                return Ok(JsValue::null());
            }
            if op == "deleteItem" {
                list.remove(i as usize);
            } else {
                return Ok(list.get(i as usize).map_or(JsValue::null(), |(saved, shown)| s(if op == "getSaveItem" { saved } else { shown })));
            }
        }
    }
    let json = serde_json::to_string(&list).map_err(|e| error(e.to_string()))?;
    if json.len() > 65_536 {
        return Err(error("XFA choice items exceed 65536 bytes"));
    }
    properties::assign(ctx, idx, "$items".into(), json)?;
    Ok(JsValue::undefined())
}
