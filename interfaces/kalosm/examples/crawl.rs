use kalosm::language::*;
use scraper::Html;
use std::collections::HashSet;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let real_visited = Arc::new(AtomicUsize::new(0));
    Page::crawl(
        Url::parse("https://dioxuslabs.com").unwrap(),
        BrowserMode::Static,
        move |page: Page| {
            let real_visited = real_visited.clone();
            Box::pin(async move {
                let visited = real_visited.fetch_add(1, Ordering::SeqCst);

                if page.url().domain() != Some("dioxuslabs.com") {
                    return CrawlFeedback::follow_none();
                }

                let Ok(document) = page.html().await else {
                    return CrawlFeedback::follow_none();
                };

                let original_length = document.html().len();

                let simplified = clean_html(document);
                let simplified_length = simplified.len();
                let percentage_decrease =
                    (original_length - simplified_length) as f32 / original_length as f32;
                println!(
                    "simplifing {} -{:.3}% from {:?} to {:?}",
                    page.url(),
                    percentage_decrease,
                    original_length,
                    simplified_length
                );

                // write the page to disk
                let _ = std::fs::create_dir_all("scraped");
                if let Ok(mut file) = std::fs::File::create(format!("scraped/{visited}.html")) {
                    _ = file.write_all(simplified.as_bytes());
                }

                CrawlFeedback::follow_all()
            }) as Pin<Box<dyn Future<Output = CrawlFeedback>>>
        },
    )
    .await
    .unwrap();
}

fn clean_html(mut html: Html) -> String {
    let mut chunker = SmartHtmlChunker::default();
    chunker.transform(&mut html);

    html.root_element().html()
}

fn element_hidden(element: &scraper::node::Element) -> bool {
    let style_hidden = match element.attr("style") {
        Some(style) => style.contains("display: none") || style.contains("display:none"),
        None => false,
    };
    let class_hidden = match element.attr("class") {
        Some(class) => class.split(' ').any(|c| c.rsplit_once(':').map_or(c, |(_, c)| c) == "hidden"),
        None => false,
    };
    style_hidden || class_hidden
}

/// Chunk HTML into paragraphs
///
/// Special elements:
/// - <p> - paragraph
/// - <h1> - heading 1
/// - <h2> - heading 2
/// - <h3> - heading 3
/// - <h4> - heading 4
/// - <h5> - heading 5
/// - <h6> - heading 6
/// - <ul> - unordered list
/// - <ol> - ordered list
/// - <li> - list item
/// - <dl> - definition list
/// - <dt> - definition term
/// - <dd> - definition description
///
/// Table elements:
/// Should be summarized
/// - <table> - table
/// - <thead> - table header
/// - <tbody> - table body
/// - <tr> - table row
/// - <td> - table cell
/// - <th> - table header cell
///
/// Special elements that need to be translated:
/// - <a> - anchor
/// - <img> - image (summarize?)
///
/// Interactive Elements to be ignored:
/// - <select> - select box
/// - <option> - select box option
/// - <form> - form
/// - <label> - form label
struct SmartHtmlChunker {
    important_attributes: HashSet<String>,
    important_elements: HashSet<String>,
    ignore_elements: HashSet<String>,
    standalone_elements: HashSet<String>,
}

impl Default for SmartHtmlChunker {
    fn default() -> Self {
        const IMPORTANT_ATTRIBUTES: &[&str] = &["title", "role", "type"];
        const IGNORE_ELEMENTS: &[&str] = &[
            "script", "style", "input", "textarea", "form", "select", "option", "label", "head",
            "link", "meta", "title", "iframe", "button",
        ];
        const IMPORTANT_ELEMENTS: &[&str] = &[
            "p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "dl", "dt", "dd", "table",
            "thead", "tbody", "tr", "td", "th", "body", "html", "pre",
        ];
        const STANDALONE_ELEMENTS: &[&str] = &[];
        Self {
            important_attributes: IMPORTANT_ATTRIBUTES.iter().map(|s| s.to_string()).collect(),
            important_elements: IMPORTANT_ELEMENTS.iter().map(|s| s.to_string()).collect(),
            ignore_elements: IGNORE_ELEMENTS.iter().map(|s| s.to_string()).collect(),
            standalone_elements: STANDALONE_ELEMENTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl SmartHtmlChunker {
    fn include_links(&mut self) {
        self.important_elements.insert("a".to_string());
        self.important_attributes.insert("href".to_string());
        self.standalone_elements.insert("a".to_string());
    }

    fn include_images(&mut self) {
        self.important_elements.insert("img".to_string());
        self.important_attributes.insert("src".to_string());
        self.important_attributes.insert("alt".to_string());
        self.standalone_elements.insert("img".to_string());
    }

    fn transform(&mut self, html: &mut Html) {
        self.transform_node(html.tree.root_mut());
    }

    fn transform_node(&mut self, mut node: ego_tree::NodeMut<'_, scraper::Node>) {
        let value = node.value();

        if let scraper::Node::Element(element) = value {
            let tag = element.name().to_lowercase();

            if element_hidden(element) {
                if let Some(next) = node.next_sibling() {
                    self.transform_node(next);
                }
                node.detach();
                return;
            } else if self.ignore_elements.contains(tag.as_str()) {
                let next_sibling_id = node.next_sibling().map(|n| n.id());
                node.detach();
                if let Some(id) = next_sibling_id {
                    self.transform_node(node.tree().get_mut(id).unwrap());
                }
                return;
            } else if self.important_elements.contains(tag.as_str()) {
                element
                    .attrs
                    .retain(|name, _| self.important_attributes.contains(&*name.local));
            } else {
                let last = move_children_to_parent(&mut node);
                if let Some(continue_id) = last {
                    let next = node.tree().get_mut(continue_id).unwrap();
                    self.transform_node(next);
                }
                return;
            }
        }
        if let scraper::Node::Comment(_) = value {
            if let Some(next) = node.next_sibling() {
                self.transform_node(next);
            }
            node.detach();
            return;
        }

        if let Some(child) = node.first_child() {
            self.transform_node(child);
        }

        if let scraper::Node::Element(element) = node.value() {
            let tag = element.name().to_lowercase();
            // If this isn't a standalone element and it doesn't have any children, we can safely remove the element. It is just noise.
            if !node.has_children() && !self.standalone_elements.contains(tag.as_str()) {
                if let Some(next) = node.next_sibling() {
                    self.transform_node(next);
                }
                node.detach();
                return;
            }
        }

        if let Some(next) = node.next_sibling() {
            self.transform_node(next);
        }
    }
}

/// Move all children of the given node to the parent node. Returns the node id to visit next.
fn move_children_to_parent(
    node: &mut ego_tree::NodeMut<'_, scraper::Node>,
) -> Option<ego_tree::NodeId> {
    enum AttatchTo {
        Parent,
        Next,
    }

    let add_spaces = if let scraper::Node::Element(element) = node.value() {
        element.name() != "span"
    } else {
        false
    };

    let (attach_id, attach_to) = match node.next_sibling() {
        Some(prev) => (prev.id(), AttatchTo::Next),
        None => {
            let parent = node.parent()?;
            (parent.id(), AttatchTo::Parent)
        }
    };

    let mut child_ids = Vec::new();
    fn add_and_continue(
        mut child: ego_tree::NodeMut<'_, scraper::Node>,
        child_ids: &mut Vec<ego_tree::NodeId>,
    ) {
        let child_id = child.id();
        child_ids.push(child_id);
        if let Some(next) = child.next_sibling() {
            add_and_continue(next, child_ids);
        }
        child.detach();
    }

    let Some(current) = node.first_child() else {
        let next = node.next_sibling().map(|n| n.id());
        node.detach();

        return next;
    };
    add_and_continue(current, &mut child_ids);
    node.detach();

    let tree = node.tree();
    for (i, &child_id) in child_ids.iter().enumerate() {
        match attach_to {
            AttatchTo::Parent => {
                let mut parent = tree.get_mut(attach_id).unwrap();
                parent.append_id(child_id);
            }
            AttatchTo::Next => {
                let mut prev = tree.get_mut(attach_id).unwrap();
                prev.insert_id_before(child_id);
            }
        }
        if add_spaces {
            if i == 0 {
                let mut child = tree.get_mut(child_id).unwrap();
                if let scraper::Node::Text(text) = child.value() {
                    if text.chars().next().filter(|c| c.is_whitespace()).is_none() {
                        let mut prev = child.prev_sibling();
                        let prev_value = prev.as_mut().map(|v| v.value());
                        if let Some(scraper::Node::Text(prev_text)) = prev_value {
                            if prev_text
                                .chars()
                                .last()
                                .filter(|c| c.is_whitespace())
                                .is_none()
                            {
                                child.insert_before(scraper::Node::Text(scraper::node::Text {
                                    text: " ".into(),
                                }));
                            }
                        }
                    }
                }
            }
            if i == child_ids.len() - 1 {
                let mut child = tree.get_mut(child_id).unwrap();
                if let scraper::Node::Text(text) = child.value() {
                    if text.chars().last().filter(|c| c.is_whitespace()).is_none() {
                        let mut next = child.next_sibling();
                        let next_value = next.as_mut().map(|v| v.value());
                        if let Some(scraper::Node::Text(next_text)) = next_value {
                            if next_text
                                .chars()
                                .next()
                                .filter(|c| c.is_whitespace())
                                .is_none()
                            {
                                child.insert_after(scraper::Node::Text(scraper::node::Text {
                                    text: " ".into(),
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    child_ids.first().copied()
}

#[test]
fn scripts_removed() {
    let html =
        r#"<p>Hello world!</p><script>console.log("Hello world!")</script><p>Hello world 2!</p>"#;
    let mut html = Html::parse_fragment(html);
    let mut chunker = SmartHtmlChunker::default();
    chunker.transform(&mut html);
    assert_eq!(
        html.root_element().html(),
        "<html><p>Hello world!</p><p>Hello world 2!</p></html>"
    );
}

#[test]
fn divs_removed() {
    let html = r#"<div>Hello world!</div>"#;
    let mut html = Html::parse_fragment(html);
    let mut chunker = SmartHtmlChunker::default();
    chunker.transform(&mut html);
    assert_eq!(html.root_element().html(), "<html>Hello world!</html>");
}

#[test]
fn spaces_added_between_removed_elements() {
    let html = r#"<div>Hello world 1!</div><div>Hello world 2!</div>"#;
    let mut html = Html::parse_fragment(html);
    let mut chunker = SmartHtmlChunker::default();
    chunker.transform(&mut html);
    assert_eq!(
        html.root_element().html(),
        "<html>Hello world 1! Hello world 2!</html>"
    );
}

#[test]
fn non_important_attributes_removed() {
    let html = r#"<p id="hello" class="world" style="color: red;">Hello world!</p>"#;
    let mut html = Html::parse_fragment(html);
    let mut chunker = SmartHtmlChunker::default();
    chunker.transform(&mut html);
    assert_eq!(
        html.root_element().html(),
        "<html><p id=\"hello\">Hello world!</p></html>"
    );
}
