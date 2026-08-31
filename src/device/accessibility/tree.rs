use super::Bounds;

#[derive(Default)]
pub(super) struct UiNode {
    pub(super) resource_id: String,
    pub(super) class_name: String,
    pub(super) text: String,
    pub(super) description: String,
    pub(super) clickable: bool,
    pub(super) scrollable: bool,
    pub(super) bounds: Option<Bounds>,
}

pub(super) fn parse_nodes(xml: &str) -> Vec<UiNode> {
    xml.split("<node ")
        .skip(1)
        .filter_map(|part| part.split_once('>').map(|value| value.0))
        .map(|tag| UiNode {
            resource_id: attribute(tag, "resource-id").unwrap_or_default(),
            class_name: attribute(tag, "class").unwrap_or_default(),
            text: attribute(tag, "text").unwrap_or_default(),
            description: attribute(tag, "content-desc").unwrap_or_default(),
            clickable: attribute(tag, "clickable").as_deref() == Some("true"),
            scrollable: attribute(tag, "scrollable").as_deref() == Some("true"),
            bounds: attribute(tag, "bounds").and_then(|value| parse_bounds(&value)),
        })
        .collect()
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let value = tag.split_once(&marker)?.1.split_once('"')?.0;
    Some(
        value
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

fn parse_bounds(value: &str) -> Option<Bounds> {
    let values: Vec<i32> = value
        .split(['[', ']', ','])
        .filter(|value| !value.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (values.len() == 4).then(|| (values[0], values[1], values[2], values[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_accessibility_nodes_and_decodes_attributes() {
        let xml = r#"<hierarchy><node text="" resource-id="com.kakao.talk:id/resend_indicator" class="android.view.View" content-desc="" clickable="false" scrollable="false" bounds="[1,2][31,42]"/><node text="A &amp; B" resource-id="android:id/button1" class="android.widget.Button" content-desc="Re-send" clickable="true" scrollable="false" bounds="[10,20][50,60]"/></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].bounds, Some((1, 2, 31, 42)));
        assert_eq!(nodes[1].text, "A & B");
        assert_eq!(nodes[1].description, "Re-send");
        assert_eq!(nodes[1].class_name, "android.widget.Button");
        assert!(nodes[1].clickable);
    }
}
