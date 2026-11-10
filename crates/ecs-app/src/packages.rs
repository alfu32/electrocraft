//! User component folders (JavaScript, hot reloaded) and SPICE component
//! packages (spec 6.3 and 8.1).

use crate::catalog::Catalog;
use ecs_model::{ComponentDef, DefKind, Subckt};
use ecs_schematic::symbols;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

/// `~/.config/electrocraft` (or the platform equivalent).
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join("electrocraft"))
}

/// Folder watched for `.js` components.
pub fn components_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("components"))
}

/// Folder holding imported SPICE packages.
pub fn packages_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("packages"))
}

/// Give a definition a symbol: keep a known one, else generate a box with
/// its pins split left and right, registered as `gen:<id>`.
pub fn ensure_symbol(mut def: ComponentDef) -> ComponentDef {
    if !def.symbol.is_empty() && symbols::exists(&def.symbol) {
        return def;
    }
    let names: Vec<&str> = def.pins.iter().map(|p| p.name.as_str()).collect();
    let half = names.len().div_ceil(2);
    let id = format!("gen:{}", def.id);
    let title: String = def.meta.name.chars().take(10).collect();
    symbols::register(symbols::boxed(&id, &title, &names[..half], &names[half..]));
    def.symbol = id;
    def
}

/// Rename a subcircuit's ports (definition pins) so they match a symbol's
/// pin names. `map[k]` is the new name of port `k`.
pub fn rename_ports(def: &ComponentDef, map: &[String]) -> ComponentDef {
    let mut d = def.clone();
    let old: Vec<String> = d.pins.iter().map(|p| p.name.clone()).collect();
    for (p, new) in d.pins.iter_mut().zip(map) {
        p.name = new.clone();
    }
    if let DefKind::Subcircuit(body) = &def.kind {
        let mut b: Subckt = (**body).clone();
        for el in &mut b.elements {
            for n in &mut el.nodes {
                if let Some(k) = old.iter().position(|o| o == n) {
                    *n = map[k].clone();
                }
            }
        }
        d.kind = DefKind::Subcircuit(Arc::new(b));
    }
    d
}

/// One component of a package and how it is drawn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PackageComponent {
    pub id: String,
    /// Symbol id; empty for a generated box.
    pub symbol: String,
    /// Symbol pin name for each port, in port order (empty: keep names).
    #[serde(default)]
    pub pins: Vec<String>,
}

/// `package.json` (spec 10, component package).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Package {
    pub format_version: u32,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    /// Original SPICE files, relative to the package folder.
    pub models: Vec<String>,
    pub components: Vec<PackageComponent>,
}

/// Apply a package's symbol choices to imported definitions.
pub fn finish_defs(
    imported: &[Arc<ComponentDef>],
    choices: &[PackageComponent],
    package: &str,
) -> Vec<ComponentDef> {
    imported
        .iter()
        .map(|d| {
            let choice = choices.iter().find(|c| c.id == d.id);
            let mut def = match choice {
                Some(c) if c.pins.len() == d.pins.len() && !c.pins.is_empty() => {
                    rename_ports(d, &c.pins)
                }
                _ => (**d).clone(),
            };
            if let Some(c) = choice
                && !c.symbol.is_empty()
            {
                def.symbol = c.symbol.clone();
            } else if matches!(def.kind, DefKind::Subcircuit(_)) {
                def.symbol.clear();
            }
            def.meta.category = format!("Imported · {package}");
            ensure_symbol(def)
        })
        .collect()
}

/// Write a package folder: `package.json` and `models/<file>`.
pub fn save_package(
    name: &str,
    file_name: &str,
    text: &str,
    components: Vec<PackageComponent>,
) -> std::io::Result<PathBuf> {
    let dir = packages_dir()
        .ok_or_else(|| std::io::Error::other("no configuration folder"))?
        .join(slug(name));
    std::fs::create_dir_all(dir.join("models"))?;
    let model = format!(
        "models/{}",
        Path::new(file_name)
            .file_name()
            .map_or("import.lib".into(), |f| f.to_string_lossy().to_string())
    );
    std::fs::write(dir.join(&model), text)?;
    let pkg = Package {
        format_version: 1,
        name: name.into(),
        version: "1.0.0".into(),
        author: String::new(),
        license: String::new(),
        models: vec![model],
        components,
    };
    std::fs::write(
        dir.join("package.json"),
        serde_json::to_string_pretty(&pkg).unwrap_or_default() + "\n",
    )?;
    Ok(dir)
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "package".into() } else { s }
}

/// Load every package folder into the catalog. Returns problems.
pub fn load_packages(cat: &mut Catalog) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(dir) = packages_dir() else {
        return problems;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return problems;
    };
    for e in entries.flatten() {
        let folder = e.path();
        let Ok(text) = std::fs::read_to_string(folder.join("package.json")) else {
            continue;
        };
        let pkg: Package = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(err) => {
                problems.push(format!("{}: {err}", folder.display()));
                continue;
            }
        };
        for m in &pkg.models {
            match ecs_spice::import_file(&folder.join(m)) {
                Ok(imp) => {
                    for d in finish_defs(&imp.defs, &pkg.components, &pkg.name) {
                        cat.lib.insert(d);
                    }
                }
                Err(err) => problems.push(format!("package {}: {err}", pkg.name)),
            }
        }
    }
    problems
}

/// Watches the components folder and reloads `.js` files when they change.
#[derive(Default)]
pub struct UserLibrary {
    stamp: BTreeMap<String, SystemTime>,
    last_check: Option<Instant>,
    ids: Vec<String>,
    pub problems: Vec<String>,
}

impl UserLibrary {
    /// Check at most once a second. Returns true when definitions changed.
    pub fn poll(&mut self, cat: &mut Catalog) -> bool {
        if self
            .last_check
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return false;
        }
        self.last_check = Some(Instant::now());
        let Some(dir) = components_dir() else {
            return false;
        };
        let stamp = ecs_script::folder_stamp(&dir);
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        for id in self.ids.drain(..) {
            cat.lib.remove(&id);
        }
        let (defs, errors) = ecs_script::load_folder(&dir);
        self.problems = errors.iter().map(|e| e.to_string()).collect();
        for d in defs {
            self.ids.push(d.id.clone());
            cat.lib.insert(ensure_symbol(d));
        }
        true
    }
}
