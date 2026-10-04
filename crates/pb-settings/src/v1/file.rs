//! Reading and editing the settings files. Edits go into the existing TOML document in place, so comments and layout
//! people wrote by hand stay (toml_edit; its whole-document `fmt()` is never called because it drops comments).

use pb_domain::{GuildId, Scope};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use super::tree::{Change, GlobalFile, SCHEMA_VERSION, ServerFile};

/// A settings file that could not be read, with where the problem is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {message}")]
pub struct FileError {
    pub file: String,
    pub message: String,
    /// Byte range of the problem in the file.
    pub span: Option<std::ops::Range<usize>>,
}

fn toml_error(file: &str, e: &toml::de::Error) -> FileError {
    FileError {
        file: file.to_owned(),
        message: e.message().to_owned(),
        span: e.span(),
    }
}

pub fn parse_global(file: &str, text: &str) -> Result<GlobalFile, FileError> {
    let parsed: GlobalFile = toml::from_str(text).map_err(|e| toml_error(file, &e))?;
    check_version(file, parsed.schema)?;
    Ok(parsed)
}

pub fn parse_server(file: &str, text: &str) -> Result<ServerFile, FileError> {
    let parsed: ServerFile = toml::from_str(text).map_err(|e| toml_error(file, &e))?;
    check_version(file, parsed.schema)?;
    Ok(parsed)
}

fn check_version(file: &str, v: u32) -> Result<(), FileError> {
    if v > SCHEMA_VERSION {
        return Err(FileError {
            file: file.to_owned(),
            message: format!(
                "written by a newer version of the bot (schema {v}, this one reads up to {SCHEMA_VERSION})"
            ),
            span: None,
        });
    }
    Ok(())
}

/// A new, empty settings file headed by `comment` (each of its lines becomes a `#` comment).
pub fn new_document(comment: &str) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["schema"] = toml_edit::value(i64::from(SCHEMA_VERSION));
    if let Some(mut key) = doc.as_table_mut().key_mut("schema") {
        let header: String = comment
            .lines()
            .map(|l| {
                if l.is_empty() {
                    "#\n".to_owned()
                } else {
                    format!("# {l}\n")
                }
            })
            .collect();
        key.leaf_decor_mut().set_prefix(header);
    }
    doc
}

/// The file a change belongs to: `None` = settings/global.toml, `Some(g)` = settings/servers/<g>.toml.
pub fn file_of(change: &Change) -> Option<GuildId> {
    change.guild()
}

fn json_to_value(v: &serde_json::Value) -> Option<Value> {
    Some(match v {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => Value::from(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::from(i),
            None => Value::from(n.as_f64()?),
        },
        serde_json::Value::String(s) => Value::from(s.as_str()),
        serde_json::Value::Array(items) => {
            let mut a = Array::new();
            for item in items {
                a.push_formatted(json_to_value(item)?);
            }
            Value::Array(a)
        }
        serde_json::Value::Object(map) => {
            let mut t = InlineTable::new();
            for (k, item) in map {
                if let Some(v) = json_to_value(item) {
                    t.insert(k, v);
                }
            }
            Value::InlineTable(t)
        }
    })
}

/// Turns an inline table (`key = { a = 1 }`, as people write by hand) into a table in place, keeping its entries, so
/// edits below it change it instead of replacing it.
fn untangle(item: &mut Item) {
    if let Some(inline) = item.as_inline_table().cloned() {
        *item = Item::Table(inline.into_table());
    }
}

/// The table at `path` below the document root, created (as implicit tables) when missing.
fn table_at<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> &'a mut Table {
    let mut t = doc.as_table_mut();
    for (i, part) in path.iter().enumerate() {
        let item = t.entry(part).or_insert_with(|| {
            let mut new = Table::new();
            new.set_implicit(i + 1 < path.len());
            Item::Table(new)
        });
        untangle(item);
        if !item.is_table() {
            let mut new = Table::new();
            new.set_implicit(i + 1 < path.len());
            *item = Item::Table(new);
        }
        t = item.as_table_mut().unwrap_or_else(|| unreachable!("just made a table"));
    }
    t
}

fn get_table<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut Table> {
    let mut t = doc.as_table_mut();
    for part in path {
        let item = t.get_mut(part)?;
        untangle(item);
        t = item.as_table_mut()?;
    }
    Some(t)
}

/// Removes tables along `path` (deepest first) that became empty.
fn prune(doc: &mut DocumentMut, path: &[&str]) {
    for depth in (1..=path.len()).rev() {
        let (parent, last) = path[..depth].split_at(depth - 1);
        let empty = get_table(doc, path[..depth].as_ref()).is_some_and(|t| t.is_empty());
        if empty && let Some(p) = get_table(doc, parent) {
            p.remove(last[0]);
        }
    }
}

fn base_path(scope: Scope) -> Vec<String> {
    match scope {
        Scope::Global | Scope::Server { .. } => vec![],
        Scope::Person { user, .. } => vec!["people".into(), user.to_string()],
    }
}

fn setting_path(scope: Scope, key: &str) -> (Vec<String>, String) {
    let mut path = base_path(scope);
    path.push("settings".into());
    if let Some(rest) = key.strip_prefix("label.")
        && let Some((label, what)) = rest.split_once('.')
    {
        path.push("labels".into());
        path.push(label.into());
        return (path, what.into());
    }
    (path, key.into())
}

/// Applies a change to the document of its file.
pub fn apply(doc: &mut DocumentMut, change: &Change) {
    match change {
        Change::Set { scope, key, after, .. } => {
            let (path, leaf) = setting_path(*scope, key);
            let path: Vec<&str> = path.iter().map(String::as_str).collect();
            let tables = after
                .as_array()
                .filter(|a| !a.is_empty() && a.iter().all(serde_json::Value::is_object));
            if let Some(items) = tables {
                // A list of records (escalation steps) reads best as [[settings.<key>]] blocks.
                let mut aot = toml_edit::ArrayOfTables::new();
                for item in items {
                    let mut t = Table::new();
                    for (k, v) in item.as_object().into_iter().flatten() {
                        if let Some(v) = json_to_value(v) {
                            t[k.as_str()] = Item::Value(v);
                        }
                    }
                    aot.push(t);
                }
                table_at(doc, &path)[leaf.as_str()] = Item::ArrayOfTables(aot);
            } else if let Some(v) = json_to_value(after) {
                table_at(doc, &path)[leaf.as_str()] = Item::Value(v);
            }
        }
        Change::Clear { scope, key, .. } => {
            let (path, leaf) = setting_path(*scope, key);
            let path: Vec<&str> = path.iter().map(String::as_str).collect();
            if let Some(t) = get_table(doc, &path) {
                t.remove(&leaf);
            }
            prune(doc, &path);
        }
        Change::Track { user, by, at, .. } => {
            let uid = user.to_string();
            let t = table_at(doc, &["people", uid.as_str()]);
            t["tracked"] = toml_edit::value(true);
            match by {
                Some(b) => t["added_by"] = toml_edit::value(b.to_string()),
                None => {
                    t.remove("added_by");
                }
            }
            t["added_at"] = toml_edit::value(at.to_string());
        }
        Change::Untrack { user, .. } => {
            let uid = user.to_string();
            if let Some(t) = get_table(doc, &["people", uid.as_str()]) {
                t.remove("tracked");
                t.remove("added_by");
                t.remove("added_at");
            }
            prune(doc, &["people", uid.as_str()]);
        }
        Change::VoiceLine { scope, line, after, .. } => {
            let mut path = base_path(*scope);
            path.push("voice_lines".into());
            let path: Vec<&str> = path.iter().map(String::as_str).collect();
            let key = line.to_string();
            match after {
                Some(slot) => {
                    let mut t = Table::new();
                    if !slot.clips.is_empty() {
                        let mut a = Array::new();
                        for c in &slot.clips {
                            a.push(c.to_string());
                        }
                        t["clips"] = Item::Value(Value::Array(a));
                    }
                    if !slot.text.is_empty() {
                        let mut it = InlineTable::new();
                        for (lang, text) in &slot.text {
                            it.insert(lang.to_string(), Value::from(text.as_str()));
                        }
                        t["text"] = Item::Value(Value::InlineTable(it));
                    }
                    let lines = table_at(doc, &path);
                    lines[key.as_str()] = Item::Table(t);
                    lines.set_implicit(true);
                }
                None => {
                    if let Some(t) = get_table(doc, &path) {
                        t.remove(&key);
                    }
                    prune(doc, &path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hand_written_inline_tables_keep_their_entries() {
        let mut doc: DocumentMut = "[settings.labels]\nharassment = { enabled = true }\n"
            .parse()
            .expect("toml");
        let t = table_at(&mut doc, &["settings", "labels", "harassment"]);
        t.insert("threshold", toml_edit::value(0.7));
        let out = doc.to_string();
        assert!(
            out.contains("enabled = true") && out.contains("threshold = 0.7"),
            "{out}"
        );
        let mut doc: DocumentMut = "[settings.labels]\nharassment = { enabled = true }\n"
            .parse()
            .expect("toml");
        get_table(&mut doc, &["settings", "labels", "harassment"]).map(|t| t.remove("enabled"));
        assert!(
            !doc.to_string().contains("enabled"),
            "a clear reaches into the inline table"
        );
    }

    use super::*;
    use crate::{SettingKey, SettingsTree};
    use pb_domain::UserId;

    const G: GuildId = GuildId(10);

    #[test]
    fn edits_keep_comments_and_read_back_the_same() {
        let text =
            "# Server settings, edited by hand too\nschema = 1\n\n[settings]\n# be strict here\nthreshold = 0.4\n";
        let mut doc: DocumentMut = text.parse().expect("toml");
        let mut tree = SettingsTree::default();
        tree.servers.insert(G, parse_server("x.toml", text).expect("parses"));
        let changes = vec![
            tree.set(
                Scope::Server { guild: G },
                SettingKey::Strikes,
                serde_json::json!(2),
                false,
            )
            .expect("ok"),
            tree.set(
                Scope::Server { guild: G },
                SettingKey::LabelEnabled(pb_domain::Label::Harassment),
                serde_json::json!(true),
                false,
            )
            .expect("ok"),
            tree.track(G, UserId(5), Some(UserId(1)), jiff::Timestamp::UNIX_EPOCH),
            tree.set(
                Scope::Person {
                    guild: G,
                    user: UserId(5),
                },
                SettingKey::Threshold,
                serde_json::json!(0.7),
                false,
            )
            .expect("ok"),
            tree.set(
                Scope::Server { guild: G },
                SettingKey::Escalation,
                serde_json::json!([{"from": 1}, {"from": 4, "action": "timeout", "duration": "10m"}]),
                false,
            )
            .expect("ok"),
            tree.set_voice_line(
                Scope::Server { guild: G },
                "greeting".parse().expect("key"),
                Some(pb_voicelines::Slot {
                    clips: vec![],
                    text: [("de".parse().expect("de"), "Hallo {name}!".to_owned())].into(),
                }),
            ),
        ];
        for c in changes.into_iter().flatten() {
            apply(&mut doc, &c);
        }
        let out = doc.to_string();
        assert!(out.contains("# be strict here"), "{out}");
        assert!(out.contains("# Server settings, edited by hand too"), "{out}");
        let back = parse_server("x.toml", &out).expect("reads back");
        assert_eq!(back, tree.servers[&G], "{out}");
        insta::assert_snapshot!(out);

        let mut tree2 = tree.clone();
        let mut clear = vec![tree2.untrack(G, UserId(5))];
        clear.push(
            tree2
                .clear(
                    Scope::Person {
                        guild: G,
                        user: UserId(5),
                    },
                    SettingKey::Threshold,
                    false,
                )
                .expect("an admin setting"),
        );
        clear.push(tree2.set_voice_line(Scope::Server { guild: G }, "greeting".parse().expect("key"), None));
        for c in clear.into_iter().flatten() {
            apply(&mut doc, &c);
        }
        let out2 = doc.to_string();
        assert_eq!(
            parse_server("x.toml", &out2).expect("reads back"),
            tree2.servers[&G],
            "{out2}"
        );
        assert!(
            !out2.contains("people"),
            "an untracked person without settings leaves no trace: {out2}"
        );
    }

    #[test]
    fn reports_where_a_file_is_wrong() {
        let e = parse_server("servers/10.toml", "schema = 1\n[settings]\nthreshold = 7\n").expect_err("invalid");
        assert!(e.message.contains("between 0 and 1"), "{e}");
        assert!(e.span.is_some());
        let e = parse_global("global.toml", "schema = 9\n").expect_err("newer");
        assert!(e.message.contains("newer version"));
    }
}
