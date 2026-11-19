//! Generation parameters stored by image generators: the user's positive and
//! negative prompt and the seed. ComfyUI stores its API graph as JSON in a PNG
//! text chunk named `prompt` (and as `prompt:{…}` in the EXIF of JPEG/WebP
//! saves); Automatic1111-style tools store a plain `parameters` text. Only the
//! prompt texts the user typed are returned, not the graph itself.
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Generation {
    pub positive: String,
    pub negative: String,
    pub seed: Option<String>,
}
impl Generation {
    /// Positive and negative prompt, then the seed, for the clipboard.
    pub fn clipboard(&self) -> String {
        let mut out = format!(
            "Positive prompt:\n{}\n\nNegative prompt:\n{}",
            self.positive, self.negative
        );
        if let Some(seed) = &self.seed {
            out.push_str(&format!("\n\nSeed: {seed}"));
        }
        out
    }
    fn is_empty(&self) -> bool {
        self.positive.is_empty() && self.negative.is_empty() && self.seed.is_none()
    }
}

/// Text chunks larger than this are skipped (workflows are well below it).
const MAX_TEXT: u32 = 16 << 20;
/// JPEG/WebP: how much of the file start is searched for an EXIF prompt.
const MAX_SCAN: u64 = 4 << 20;

pub fn read(path: &Path) -> Option<Generation> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic).ok()?;
    let texts = if magic == *b"\x89PNG\r\n\x1a\n" {
        png_texts(&mut file)
    } else {
        file.seek(SeekFrom::Start(0)).ok()?;
        let mut head = Vec::new();
        file.take(MAX_SCAN).read_to_end(&mut head).ok()?;
        embedded_texts(&head)
    };
    from_texts(&texts)
}

/// Generation parameters from text entries (keyword → text).
pub fn from_texts(texts: &BTreeMap<String, String>) -> Option<Generation> {
    let workflow = texts
        .get("workflow")
        .and_then(|t| serde_json::from_str::<Value>(t).ok());
    if let Some(found) = texts
        .get("prompt")
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .map(|graph| with_titles(graph, workflow.as_ref()))
        .and_then(|graph| from_graph(&graph))
    {
        return Some(found);
    }
    texts
        .get("parameters")
        .and_then(|t| from_parameters(t))
        .filter(|g| !g.is_empty())
}

fn png_texts(file: &mut File) -> BTreeMap<String, String> {
    let mut texts = BTreeMap::new();
    let mut header = [0u8; 8];
    while file.read_exact(&mut header).is_ok() {
        let len = u32::from_be_bytes(header[..4].try_into().unwrap());
        let kind = &header[4..8];
        if kind == b"IEND" {
            break;
        }
        let text = matches!(kind, b"tEXt" | b"iTXt" | b"zTXt");
        if text && len <= MAX_TEXT {
            let mut body = vec![0; len as usize];
            if file.read_exact(&mut body).is_err() {
                break;
            }
            if let Some((key, value)) = text_chunk(kind, &body) {
                texts.entry(key).or_insert(value);
            }
            if file.seek(SeekFrom::Current(4)).is_err() {
                break;
            }
        } else if file.seek(SeekFrom::Current(i64::from(len) + 4)).is_err() {
            break;
        }
    }
    texts
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(u64::from(MAX_TEXT))
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn text_chunk(kind: &[u8], body: &[u8]) -> Option<(String, String)> {
    let z = body.iter().position(|b| *b == 0)?;
    let key = String::from_utf8_lossy(&body[..z]).into_owned();
    let rest = &body[z + 1..];
    let value = match kind {
        // Latin-1 by the standard; ComfyUI writes UTF-8 there anyway.
        b"tEXt" => String::from_utf8_lossy(rest).into_owned(),
        b"zTXt" => String::from_utf8_lossy(&inflate(rest.get(1..)?)?).into_owned(),
        _ => {
            let compressed = *rest.first()? == 1;
            let rest = rest.get(2..)?;
            let lang = rest.iter().position(|b| *b == 0)?;
            let rest = &rest[lang + 1..];
            let label = rest.iter().position(|b| *b == 0)?;
            let text = &rest[label + 1..];
            if compressed {
                String::from_utf8_lossy(&inflate(text)?).into_owned()
            } else {
                String::from_utf8_lossy(text).into_owned()
            }
        }
    };
    Some((key, value))
}

/// ComfyUI JPEG/WebP saves keep `prompt:{json}` in an EXIF string; some tools
/// keep Automatic1111 `parameters` text in the EXIF user comment.
fn embedded_texts(data: &[u8]) -> BTreeMap<String, String> {
    let mut texts = BTreeMap::new();
    for key in ["prompt", "Prompt"] {
        let needle = format!("{key}:{{");
        if let Some(at) = find(data, needle.as_bytes()) {
            let json = &data[at + key.len() + 1..];
            let mut stream = serde_json::Deserializer::from_slice(json).into_iter::<Value>();
            if let Some(Ok(value)) = stream.next() {
                texts.insert("prompt".into(), value.to_string());
                break;
            }
        }
    }
    if let Some(at) = find(data, b"Negative prompt:") {
        // UserComment: an 8-byte charset header, then ASCII or UTF-16.
        let start = data[..at]
            .windows(8)
            .rposition(|w| w == b"ASCII\0\0\0" || w == b"UNICODE\0")
            .map_or(at, |p| p + 8);
        let end = data[at..]
            .iter()
            .position(|b| *b == 0)
            .map_or(data.len(), |e| at + e);
        texts.insert(
            "parameters".into(),
            String::from_utf8_lossy(&data[start..end]).into_owned(),
        );
    }
    texts
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `positive\nNegative prompt: negative\nSteps: …, Seed: 123, …`
pub fn from_parameters(text: &str) -> Option<Generation> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let settings_at = text
        .rfind("\nSteps:")
        .or_else(|| text.starts_with("Steps:").then_some(0));
    let (prompts, settings) = match settings_at {
        Some(at) => (&text[..at], &text[at..]),
        None => (text, ""),
    };
    let (positive, negative) = match prompts.find("Negative prompt:") {
        Some(at) => (&prompts[..at], &prompts[at + "Negative prompt:".len()..]),
        None => (prompts, ""),
    };
    let seed = settings
        .split(',')
        .filter_map(|part| part.trim().strip_prefix("Seed:"))
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty());
    Some(Generation {
        positive: positive.trim().to_string(),
        negative: negative.trim().to_string(),
        seed,
    })
}

type Graph = Map<String, Value>;

/// Which prompt a link leads to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Positive,
    Negative,
}
impl Side {
    /// Fields that hold this side's prompt on nodes that take both
    /// (Efficient Loader, prompt stylers, ControlNet apply, guiders…).
    fn keys(self) -> &'static [&'static str] {
        match self {
            Side::Positive => &[
                "positive",
                "text_positive",
                "positive_prompt",
                "prompt_positive",
                "positive_text",
                "pos",
            ],
            Side::Negative => &[
                "negative",
                "text_negative",
                "negative_prompt",
                "prompt_negative",
                "negative_text",
                "neg",
            ],
        }
    }
    /// Nodes with both a positive and a negative output give the positive
    /// one first.
    fn of_output(output: u64) -> Self {
        if output == 1 {
            Side::Negative
        } else {
            Side::Positive
        }
    }
}

struct Node<'a> {
    class: &'a str,
    title: &'a str,
    inputs: &'a Map<String, Value>,
    output: u64,
}
fn node<'a>(graph: &'a Graph, link: &Value) -> Option<Node<'a>> {
    let pair = link.as_array().filter(|a| a.len() == 2)?;
    let id = match &pair[0] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let output = pair[1].as_u64()?;
    let entry = graph.get(&id)?;
    Some(Node {
        class: entry
            .get("class_type")
            .and_then(Value::as_str)
            .unwrap_or(""),
        title: entry
            .get("_meta")
            .and_then(|m| m.get("title"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        inputs: entry.get("inputs")?.as_object()?,
        output,
    })
}

fn is_link(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|a| a.len() == 2 && a[1].is_u64() && (a[0].is_string() || a[0].is_u64()))
}

/// Has both a positive and a negative field (so one side must be chosen).
fn two_sided(inputs: &Map<String, Value>) -> bool {
    [Side::Positive, Side::Negative]
        .iter()
        .all(|side| side.keys().iter().any(|k| inputs.contains_key(*k)))
}

/// A literal value, or the value a link leads to through primitive / string
/// nodes (`Primitive`, `String Literal`, `Seed`, text concatenation, prompt
/// stylers…).
fn scalar(graph: &Graph, value: &Value, keys: &[&str], depth: usize) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        v if is_link(v) && depth < 16 => {
            let node = node(graph, v)?;
            let inputs = node.inputs;
            // A styler's positive / negative string outputs.
            if two_sided(inputs) {
                let side = Side::of_output(node.output);
                return side
                    .keys()
                    .iter()
                    .find_map(|k| inputs.get(*k))
                    .and_then(|v| scalar(graph, v, keys, depth + 1));
            }
            for key in keys {
                if let Some(found) = inputs
                    .get(*key)
                    .and_then(|v| scalar(graph, v, keys, depth + 1))
                {
                    return Some(found);
                }
            }
            // Text joining nodes: text_a / text_b / string_1 …
            let parts: Vec<String> = inputs
                .iter()
                .filter(|(k, _)| k.starts_with("text") || k.starts_with("string"))
                .filter_map(|(_, v)| scalar(graph, v, keys, depth + 1))
                .filter(|s| !s.is_empty())
                .collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        _ => None,
    }
}

const TEXT_KEYS: [&str; 9] = [
    "populated_text",
    "text",
    "string",
    "value",
    "prompt",
    "t5xxl",
    "clip_l",
    "text_g",
    "wildcard_text",
];
/// Prompt fields of text encoders, in order of preference. A wildcard
/// node's `populated_text` is what was really used; `wildcard_text` is
/// the template.
const PROMPT_FIELDS: [&str; 8] = [
    "populated_text",
    "text",
    "text_g",
    "text_l",
    "clip_l",
    "t5xxl",
    "prompt",
    "wildcard_text",
];
const SEED_KEYS: [&str; 4] = ["seed", "noise_seed", "value", "int"];

fn push(out: &mut Vec<String>, text: String) {
    let text = text.trim().to_string();
    if !text.is_empty() && !out.contains(&text) {
        out.push(text);
    }
}

/// Prompt texts reaching a conditioning input of one side.
fn texts(graph: &Graph, link: &Value, side: Side, depth: usize, out: &mut Vec<String>) {
    if depth > 24 {
        return;
    }
    let Some(node) = node(graph, link) else {
        return;
    };
    let inputs = node.inputs;
    // A zeroed conditioning (Flux negative) has no text of its own.
    if node.class.contains("ZeroOut") {
        return;
    }
    let mut found = false;
    for key in PROMPT_FIELDS {
        if key == "wildcard_text" && found {
            break;
        }
        if let Some(text) = inputs
            .get(key)
            .and_then(|v| scalar(graph, v, &TEXT_KEYS, 0))
        {
            found = true;
            push(out, text);
        }
    }
    if found {
        return;
    }
    // Both sides on one node: take this side's field (a text, or a link to
    // a text or to more conditioning).
    if let Some(value) = side.keys().iter().find_map(|k| inputs.get(*k)) {
        if let Some(text) = value.as_str() {
            push(out, text.to_string());
        } else if is_link(value) {
            let before = out.len();
            texts(graph, value, side, depth + 1, out);
            if out.len() == before {
                if let Some(text) = scalar(graph, value, &TEXT_KEYS, 0) {
                    push(out, text);
                }
            }
        }
        return;
    }
    let mut followed = false;
    for (key, value) in inputs {
        if key.contains("conditioning") && is_link(value) {
            followed = true;
            texts(graph, value, side, depth + 1, out);
        }
    }
    if followed {
        return;
    }
    // Unknown encoder: any text-like field (never a system node's).
    let title = node.title;
    for (key, value) in inputs {
        if is_system_node(key, title, node.class) {
            continue;
        }
        let key = key.to_ascii_lowercase();
        if (key.contains("text") || key.contains("prompt"))
            && !key.contains("negative")
            && !key.contains("system")
            && !key.contains("template")
        {
            if let Some(text) = scalar(graph, value, &TEXT_KEYS, 0) {
                push(out, text);
            }
        }
    }
}

/// Prompt-like field names: used when the structured walk finds nothing.
const PROMPT_KEYS: [&str; 19] = [
    "text",
    "prompt",
    "positive",
    "negative",
    "positive_prompt",
    "negative_prompt",
    "caption",
    "instruction",
    "edit_prompt",
    "user_prompt",
    "prompt_text",
    "text_positive",
    "text_negative",
    "image_prompt",
    "description",
    "input_text",
    "input_prompt",
    "main_prompt",
    "scene_prompt",
];
const MODEL_FILES: [&str; 5] = [".safetensors", ".ckpt", ".gguf", ".pt", ".pth"];

/// Node titles from the saved workflow for nodes whose API entry has none
/// (older ComfyUI versions); titles tell a "System Prompt" node from a
/// "User Prompt" one.
fn with_titles(mut graph: Value, workflow: Option<&Value>) -> Value {
    let Some(nodes) = workflow
        .and_then(|w| w.get("nodes"))
        .and_then(Value::as_array)
    else {
        return graph;
    };
    let Some(map) = graph.as_object_mut() else {
        return graph;
    };
    for node in nodes {
        let (Some(id), Some(title)) = (
            node.get("id").map(|i| match i {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }),
            node.get("title").and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(entry) = map.get_mut(&id).and_then(Value::as_object_mut) {
            let meta = entry
                .entry("_meta")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(meta) = meta.as_object_mut() {
                meta.entry("title")
                    .or_insert_with(|| Value::String(title.to_string()));
            }
        }
    }
    graph
}

/// Lower-case words of a label (`user_prompt`, `UserPrompt`, `user-prompt`).
fn words(label: &str) -> Vec<String> {
    let mut spaced = String::new();
    let mut previous: Option<char> = None;
    for c in label.chars() {
        if c.is_uppercase() && previous.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()) {
            spaced.push(' ');
        }
        spaced.push(if c.is_alphanumeric() { c } else { ' ' });
        previous = Some(c);
    }
    spaced
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}
const USER_KEYS: [&str; 6] = [
    "user prompt",
    "prompt user",
    "user text",
    "user input",
    "user instruction",
    "user message",
];
/// The user's own text (not an enhancer's or a system instruction).
fn is_user_field(key: &str, title: &str) -> bool {
    let key = words(key).join(" ");
    let title = words(title).join(" ");
    if USER_KEYS.contains(&key.as_str()) {
        return true;
    }
    if title.split(' ').any(|w| w == "system") {
        return false;
    }
    [
        "original prompt",
        "input prompt",
        "main prompt",
        "scene prompt",
    ]
    .contains(&key.as_str())
        || ["user prompt", "original prompt", "user input"]
            .iter()
            .any(|t| title.contains(t))
}
/// A system/instruction node, whatever its field is called.
fn is_system_node(key: &str, title: &str, class: &str) -> bool {
    if words(key).iter().any(|w| w == "system") {
        return true;
    }
    [title, class].iter().any(|label| {
        let w = words(label);
        w.iter().any(|w| w == "system") && !w.iter().any(|w| w == "user" || w == "negative")
    })
}

struct Field {
    key: String,
    text: String,
    user: bool,
}
/// Text fields of one node that look like a prompt.
fn prompt_fields(node: &Value) -> Vec<Field> {
    let class_name = node.get("class_type").and_then(Value::as_str).unwrap_or("");
    let class = class_name.to_ascii_lowercase();
    let title = node
        .get("_meta")
        .and_then(|m| m.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(inputs) = node.get("inputs").and_then(Value::as_object) else {
        return vec![];
    };
    let mut out = vec![];
    for (key, value) in inputs {
        let key = key.to_ascii_lowercase();
        if key.contains("template") || key == "filename_prefix" {
            continue;
        }
        let user = is_user_field(&key, title);
        if !user && is_system_node(&key, title, class_name) {
            continue;
        }
        let allowed = PROMPT_KEYS.contains(&key.as_str())
            || key.ends_with("_prompt")
            || key.ends_with("_text")
            || ((key == "string" || key == "value")
                && ["text", "string", "prompt"]
                    .iter()
                    .any(|t| class.contains(t)));
        let Some(text) = value.as_str() else { continue };
        let lower = text.trim().to_ascii_lowercase();
        if !allowed || lower.is_empty() || text.len() > 150_000 {
            continue;
        }
        if MODEL_FILES.iter().any(|e| lower.ends_with(e)) {
            continue;
        }
        out.push(Field {
            key,
            text: text.trim().to_string(),
            user,
        });
    }
    out
}
/// When the user's own prompt is recognised, the positive side is only that.
fn prefer_user(fields: Vec<Field>) -> Vec<String> {
    let only_user = fields.iter().any(|f| f.user);
    let mut out = vec![];
    for field in fields.into_iter().filter(|f| f.user || !only_user) {
        push(&mut out, field.text);
    }
    out
}

/// Every node a link leads back to, nearest first.
fn upstream(graph: &Graph, link: Option<&Value>) -> Vec<String> {
    let mut seen: Vec<String> = vec![];
    let mut queue = std::collections::VecDeque::new();
    if let Some(id) = link.filter(|l| is_link(l)).and_then(|l| l.get(0)) {
        queue.push_back(match id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    }
    while let Some(id) = queue.pop_front() {
        if seen.contains(&id) || seen.len() >= 5000 {
            continue;
        }
        let Some(inputs) = graph
            .get(&id)
            .and_then(|n| n.get("inputs"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        seen.push(id);
        for value in inputs.values() {
            if is_link(value) {
                queue.push_back(match &value[0] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
            }
        }
    }
    seen
}

/// Fallback: prompt fields of the nodes behind one side of a sampler (and not
/// behind the other side).
fn side_fields(graph: &Graph, side: &[String], other: &[String], negative: bool) -> Vec<String> {
    let mut fields = vec![];
    for id in side.iter().filter(|id| !other.contains(id)) {
        for field in prompt_fields(&graph[id]) {
            let says_negative = field.key.contains("negative") || field.key.starts_with("neg");
            if says_negative == negative {
                fields.push(field);
            }
        }
    }
    if negative {
        let mut out = vec![];
        for field in fields {
            push(&mut out, field.text);
        }
        out
    } else {
        prefer_user(fields)
    }
}

fn numeric_id(id: &str) -> (u64, &str) {
    let head = id.split(':').next().unwrap_or(id);
    (head.parse().unwrap_or(u64::MAX), id)
}

/// The first sampler of a ComfyUI API graph (by node id), its prompts and seed.
pub fn from_graph(graph: &Value) -> Option<Generation> {
    let graph = graph.as_object()?;
    let mut ids: Vec<&String> = graph.keys().collect();
    ids.sort_by(|a, b| numeric_id(a).cmp(&numeric_id(b)));
    let mut fallback = None;
    for id in ids {
        let Some(inputs) = graph[id].get("inputs").and_then(Value::as_object) else {
            continue;
        };
        // Only samplers: guiders and ControlNet nodes also take prompts.
        let class = graph[id]
            .get("class_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let sampler = class.contains("Sampler")
            || ["seed", "noise_seed", "noise"]
                .iter()
                .any(|k| inputs.contains_key(*k));
        if !sampler {
            continue;
        }
        // SamplerCustomAdvanced: the guider holds the prompts, the noise the seed.
        let guider = inputs
            .get("guider")
            .and_then(|g| node(graph, g))
            .map(|g| g.inputs);
        let source = guider.unwrap_or(inputs);
        let Some(positive_link) = source
            .get("positive")
            .or_else(|| source.get("conditioning"))
        else {
            continue;
        };
        if !is_link(positive_link) {
            continue;
        }
        let mut positive = vec![];
        texts(graph, positive_link, Side::Positive, 0, &mut positive);
        let mut negative = vec![];
        if let Some(link) = source.get("negative") {
            texts(graph, link, Side::Negative, 0, &mut negative);
        }
        // Unknown nodes (MOSS-Image, prompt enhancers…): take the prompt
        // fields of the nodes behind each side instead.
        if positive.is_empty() || negative.is_empty() {
            let behind_positive = upstream(graph, Some(positive_link));
            let behind_negative = upstream(graph, source.get("negative"));
            if positive.is_empty() {
                positive = side_fields(graph, &behind_positive, &behind_negative, false);
            }
            if negative.is_empty() {
                negative = side_fields(graph, &behind_negative, &behind_positive, true);
            }
        }
        let seed_source = inputs
            .get("noise")
            .and_then(|n| node(graph, n))
            .map_or(inputs, |n| n.inputs);
        let seed = ["seed", "noise_seed"]
            .iter()
            .find_map(|k| seed_source.get(*k))
            .and_then(|v| scalar(graph, v, &SEED_KEYS, 0));
        let found = Generation {
            positive: positive.join("\n"),
            negative: negative.join("\n"),
            seed,
        };
        if found.positive.is_empty() {
            fallback = fallback.or(Some(found).filter(|f| !f.is_empty()));
            continue;
        }
        return Some(found);
    }
    fallback.or_else(|| whole_graph(graph))
}

/// No usable sampler: prompt fields of every node, negative by name.
fn whole_graph(graph: &Graph) -> Option<Generation> {
    let mut ids: Vec<&String> = graph.keys().collect();
    ids.sort_by(|a, b| numeric_id(a).cmp(&numeric_id(b)));
    let (mut positive, mut negative, mut seed) = (vec![], vec![], None::<String>);
    for id in ids {
        let node = &graph[id];
        let title = node
            .get("_meta")
            .and_then(|m| m.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        for field in prompt_fields(node) {
            if field.key.contains("negative") || title.contains("negative") {
                push(&mut negative, field.text);
            } else {
                positive.push(field);
            }
        }
        if seed.is_none() {
            seed = node
                .get("inputs")
                .and_then(|i| i.get("seed").or_else(|| i.get("noise_seed")))
                .and_then(|v| scalar(graph, v, &SEED_KEYS, 0));
        }
    }
    let found = Generation {
        positive: prefer_user(positive).join("\n"),
        negative: negative.join("\n"),
        seed,
    };
    (!found.is_empty()).then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png_with(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut put = |kind: &[u8; 4], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let mut crc = crc32fast::Hasher::new();
            crc.update(kind);
            crc.update(data);
            out.extend_from_slice(&crc.finalize().to_be_bytes());
        };
        put(b"IHDR", &[0; 13]);
        for (kind, data) in chunks {
            put(kind, data);
        }
        put(b"IDAT", &[1, 2, 3]);
        put(b"IEND", &[]);
        out
    }

    fn basic_graph() -> Value {
        json!({
            "3": {"class_type": "KSampler", "inputs": {
                "seed": 156680208700286u64, "steps": 20,
                "positive": ["6", 0], "negative": ["7", 0], "model": ["4", 0]}},
            "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "x.safetensors"}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "a cat on a sofa", "clip": ["4", 1]}},
            "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "blurry, text", "clip": ["4", 1]}}
        })
    }

    #[test]
    fn reads_prompts_and_seed_from_a_comfyui_png_and_ignores_the_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ComfyUI_00001_.png");
        let mut prompt = b"prompt\0".to_vec();
        prompt.extend_from_slice(basic_graph().to_string().as_bytes());
        let mut workflow = b"workflow\0".to_vec();
        workflow.extend_from_slice(br#"{"nodes":[]}"#);
        std::fs::write(&path, png_with(&[(b"tEXt", prompt), (b"tEXt", workflow)])).unwrap();
        let g = read(&path).unwrap();
        assert_eq!(g.positive, "a cat on a sofa");
        assert_eq!(g.negative, "blurry, text");
        assert_eq!(g.seed.as_deref(), Some("156680208700286"));
        assert_eq!(
            g.clipboard(),
            "Positive prompt:\na cat on a sofa\n\nNegative prompt:\nblurry, text\n\nSeed: 156680208700286"
        );
    }

    #[test]
    fn compressed_itxt_and_parameters_text_are_read() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(
            b"red fox\nNegative prompt: ugly\nSteps: 30, Sampler: Euler, Seed: 42, Size: 512x512",
        )
        .unwrap();
        let mut body = b"parameters\0\x01\0\0\0".to_vec();
        body.extend_from_slice(&z.finish().unwrap());
        std::fs::write(&path, png_with(&[(b"iTXt", body)])).unwrap();
        let g = read(&path).unwrap();
        assert_eq!(
            (g.positive.as_str(), g.negative.as_str()),
            ("red fox", "ugly")
        );
        assert_eq!(g.seed.as_deref(), Some("42"));
    }

    #[test]
    fn follows_links_through_primitives_guiders_and_conditioning_nodes() {
        let graph = json!({
            "10": {"class_type": "PrimitiveString", "inputs": {"value": "portrait, soft light"}},
            "11": {"class_type": "CLIPTextEncode", "inputs": {"text": ["10", 0]}},
            "12": {"class_type": "FluxGuidance", "inputs": {"conditioning": ["11", 0], "guidance": 3.5}},
            "13": {"class_type": "CLIPTextEncode", "inputs": {"text": "lowres"}},
            "14": {"class_type": "ControlNetApplyAdvanced", "inputs": {"positive": ["12", 0], "negative": ["13", 0]}},
            "15": {"class_type": "CFGGuider", "inputs": {"positive": ["14", 0], "negative": ["14", 1]}},
            "16": {"class_type": "RandomNoise", "inputs": {"noise_seed": ["17", 0]}},
            "17": {"class_type": "Seed", "inputs": {"seed": 777}},
            "20": {"class_type": "SamplerCustomAdvanced", "inputs": {"noise": ["16", 0], "guider": ["15", 0]}}
        });
        let g = from_graph(&graph).unwrap();
        assert_eq!(g.positive, "portrait, soft light");
        assert_eq!(g.negative, "lowres");
        assert_eq!(g.seed.as_deref(), Some("777"));
    }

    #[test]
    fn prompts_in_loader_styler_and_wildcard_nodes_are_found() {
        // Efficient Loader: the prompts are plain fields of the loader.
        let efficient = json!({
            "4": {"class_type": "Efficient Loader", "inputs": {
                "ckpt_name": "x", "positive": "moss on old stones, macro", "negative": "blurry"}},
            "5": {"class_type": "KSampler (Efficient)", "inputs": {
                "seed": 42, "positive": ["4", 1], "negative": ["4", 2], "model": ["4", 0]}}
        });
        let g = from_graph(&efficient).unwrap();
        assert_eq!(
            (g.positive.as_str(), g.negative.as_str(), g.seed.as_deref()),
            ("moss on old stones, macro", "blurry", Some("42"))
        );
        // Prompt styler: string outputs 0 = positive, 1 = negative.
        let styler = json!({
            "1": {"class_type": "SDXLPromptStyler", "inputs": {
                "text_positive": "moss forest", "text_negative": "ugly", "style": "photo"}},
            "2": {"class_type": "CLIPTextEncode", "inputs": {"text": ["1", 0]}},
            "3": {"class_type": "CLIPTextEncode", "inputs": {"text": ["1", 1]}},
            "9": {"class_type": "KSampler", "inputs": {"seed": 7, "positive": ["2", 0], "negative": ["3", 0]}}
        });
        let g = from_graph(&styler).unwrap();
        assert_eq!(
            (g.positive.as_str(), g.negative.as_str()),
            ("moss forest", "ugly")
        );
        // Wildcards: the populated text, not the template; Flux zeroed negative.
        let wildcard = json!({
            "1": {"class_type": "ImpactWildcardEncode", "inputs": {
                "wildcard_text": "__plants__ on rocks", "populated_text": "moss on rocks"}},
            "2": {"class_type": "ConditioningZeroOut", "inputs": {"conditioning": ["1", 0]}},
            "3": {"class_type": "KSampler", "inputs": {"seed": 1, "positive": ["1", 0], "negative": ["2", 0]}}
        });
        let g = from_graph(&wildcard).unwrap();
        assert_eq!(
            (g.positive.as_str(), g.negative.as_str()),
            ("moss on rocks", "")
        );
    }

    #[test]
    fn unknown_prompt_nodes_are_read_from_the_fields_behind_each_side() {
        // A MOSS-Image style graph: the positive text sits in a field the
        // structured walk does not know, next to a system prompt.
        let moss = json!({
            "2": {"class_type": "MossImagePromptEnhancer", "inputs": {
                "instruction": "a quiet lake at dawn, mist",
                "system_prompt": "You are a helpful prompt writer.",
                "model": "moss-image.safetensors"}},
            "3": {"class_type": "MossImageEncode", "inputs": {"enhanced": ["2", 0], "clip": ["9", 0]}},
            "4": {"class_type": "CLIPTextEncode", "inputs": {"text": "lowres, watermark"}},
            "9": {"class_type": "CLIPLoader", "inputs": {"clip_name": "t5.safetensors"}},
            "10": {"class_type": "KSampler", "inputs": {"seed": 1234, "positive": ["3", 0], "negative": ["4", 0]}}
        });
        let g = from_graph(&moss).unwrap();
        assert_eq!(g.positive, "a quiet lake at dawn, mist");
        assert_eq!(g.negative, "lowres, watermark");
        assert_eq!(g.seed.as_deref(), Some("1234"));
        // No sampler at all: every prompt field, negative by name.
        let single = json!({
            "1": {"class_type": "MossImageGenerate", "inputs": {
                "prompt": "red fox in snow", "negative_prompt": "blurry", "seed": 9}}
        });
        let g = from_graph(&single).unwrap();
        assert_eq!(
            (g.positive.as_str(), g.negative.as_str(), g.seed.as_deref()),
            ("red fox in snow", "blurry", Some("9"))
        );
    }

    #[test]
    fn only_the_users_prompt_is_taken_not_a_system_prompt() {
        // Two string nodes feed an enhancer: titled "System Prompt" and
        // "User Prompt", both with a plain `text` field.
        let graph = json!({
            "1": {"class_type": "PrimitiveStringMultiline", "_meta": {"title": "System Prompt"},
                  "inputs": {"value": "You are an expert prompt writer. Expand the idea."}},
            "2": {"class_type": "PrimitiveStringMultiline", "_meta": {"title": "User Prompt"},
                  "inputs": {"value": "a mossy stone bridge in fog"}},
            "3": {"class_type": "MossImagePromptEnhancer", "inputs": {
                  "system": ["1", 0], "instruction": ["2", 0], "enhanced_text": "a mossy stone bridge in fog, cinematic, ultra detailed"}},
            "4": {"class_type": "MossImageEncode", "inputs": {"prompt": ["3", 0]}},
            "5": {"class_type": "CLIPTextEncode", "inputs": {"text": "lowres"}},
            "6": {"class_type": "KSampler", "inputs": {"seed": 3, "positive": ["4", 0], "negative": ["5", 0]}}
        });
        let g = from_graph(&graph).unwrap();
        assert_eq!(g.positive, "a mossy stone bridge in fog");
        assert_eq!(g.negative, "lowres");
        // Titles from the saved workflow when the API graph has none.
        let mut bare = graph.clone();
        for id in ["1", "2"] {
            bare[id].as_object_mut().unwrap().remove("_meta");
        }
        let texts = BTreeMap::from([
            ("prompt".to_string(), bare.to_string()),
            (
                "workflow".to_string(),
                json!({"nodes": [{"id": 1, "title": "System Prompt"}, {"id": 2, "title": "User Prompt"}]})
                    .to_string(),
            ),
        ]);
        assert_eq!(
            from_texts(&texts).unwrap().positive,
            "a mossy stone bridge in fog"
        );
        // Without any user label, ordinary positive text is still kept, but
        // a node titled as a system prompt is not.
        assert_eq!(words("UserPrompt"), ["user", "prompt"]);
        assert!(is_system_node("text", "System Instruction", "StringNode"));
        assert!(!is_system_node(
            "text",
            "User + System prompt",
            "StringNode"
        ));
    }

    #[test]
    fn exif_prompt_in_a_jpeg_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        let mut data = b"\xff\xd8\xff\xe1\0\x10Exif\0\0MM\0*Model\0prompt:".to_vec();
        data.extend_from_slice(basic_graph().to_string().as_bytes());
        data.extend_from_slice(b"\0\xff\xd9");
        std::fs::write(&path, data).unwrap();
        assert_eq!(read(&path).unwrap().negative, "blurry, text");
    }

    #[test]
    fn images_without_generation_data_give_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.png");
        std::fs::write(&path, png_with(&[])).unwrap();
        assert_eq!(read(&path), None);
        assert_eq!(from_graph(&json!({"1": {"inputs": {"x": 1}}})), None);
    }
}
