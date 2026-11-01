//! Channel-neutral, local-only sticker resources. No network, tools, or executable skills.
//! Pack format: `pack.json`, static `SKILL.md`, `stickers.json`, and relative `assets/` files.

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path};

#[derive(RustEmbed)]
#[folder = "../../resource-packs/"]
struct Bundled;

#[derive(Clone, Debug, Serialize)]
pub struct AssetView {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PackView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sticker_count: usize,
    pub skill: String,
    pub attribution: String,
    pub assets: Vec<AssetView>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ResourceRef {
    pub pack_id: String,
    pub asset_id: String,
    pub name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResourceChunk {
    Text(String),
    Sticker(ResourceRef),
}

pub struct ResourceAsset {
    pub bytes: Bytes,
    pub mime: &'static str,
}

#[derive(Deserialize)]
struct PackManifest {
    id: String,
    name: String,
    description: String,
    attribution: String,
}

#[derive(Deserialize)]
struct StickerMetadata {
    id: String,
    name: String,
    file: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    emotion: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, rename = "use")]
    usage: String,
    #[serde(default)]
    dir: String,
}

struct Sticker {
    reference: ResourceRef,
    aliases: BTreeSet<String>,
    metadata: StickerMetadata,
    bytes: Bytes,
    mime: &'static str,
}

struct Pack {
    view: PackView,
    stickers: BTreeMap<String, Sticker>,
}

pub struct ResourceCatalog {
    packs: BTreeMap<String, Pack>,
}

impl ResourceCatalog {
    /// Load once. Built-ins are embedded even in debug builds; custom packs are read into memory.
    /// Symlinks and parent traversal are rejected using anchored, no-follow file descriptors.
    pub fn load(custom_dir: &Path) -> Result<Self> {
        let mut catalog = Self {
            packs: BTreeMap::new(),
        };
        for id in ["genui", "nekowhale", "nekowhale-gpt"] {
            let pack = load_pack(id, |relative| {
                Bundled::get(&format!("{id}/{relative}"))
                    .map(|file| file.data)
                    .with_context(|| format!("Missing embedded resource: {id}/{relative}"))
            })?;
            catalog.packs.insert(id.to_owned(), pack);
        }
        let Some(root) = open_directory(custom_dir)? else {
            return Ok(catalog);
        };
        let mut entries = std::fs::read_dir(custom_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Symlink in skills directory: {:?}",
                entry.file_name()
            );
            if !kind.is_dir() {
                continue;
            }
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Non-UTF8 pack ID"))?;
            ensure!(valid_id(&id), "Invalid resource pack ID: {id}");
            ensure!(
                !catalog.packs.contains_key(&id),
                "Duplicate resource pack ID: {id}"
            );
            let folder = open_child_dir(&root, Path::new(&id))?;
            let pack = load_pack(&id, |relative| {
                let file = open_relative_file(&folder, Path::new(relative))?;
                let limit = if relative.starts_with("assets/") {
                    32 * 1024 * 1024
                } else {
                    16 * 1024 * 1024
                };
                let mut bytes = Vec::new();
                file.take(limit + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= limit,
                    "Resource file exceeds size limit: {relative}"
                );
                Ok(Cow::Owned(bytes))
            })
            .with_context(|| format!("Loading custom resource pack {id}"))?;
            catalog.packs.insert(id, pack);
        }
        Ok(catalog)
    }

    pub fn packs(&self) -> Vec<PackView> {
        self.packs.values().map(|pack| pack.view.clone()).collect()
    }

    pub fn contains_pack(&self, id: &str) -> bool {
        self.packs.contains_key(id)
    }

    pub fn asset(&self, pack: &str, id: &str) -> Option<ResourceAsset> {
        let sticker = self.packs.get(pack)?.stickers.get(id)?;
        Some(ResourceAsset {
            bytes: sticker.bytes.clone(),
            mime: sticker.mime,
        })
    }

    fn authorized(&self, allowed: &[String]) -> Result<Vec<&Pack>> {
        allowed
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|id| {
                self.packs
                    .get(id)
                    .with_context(|| format!("Unknown resource pack: {id}"))
            })
            .collect()
    }

    /// A stable, sorted prompt independent of caller ordering. No directories are executed.
    pub fn prompt(&self, allowed: &[String], identity: &str, frequency: &str) -> Result<String> {
        let identity = match identity {
            "self" => {
                "self：可以把贴合角色的表情作为自己的反应；始终遵守角色原有身份，不因素材包变成大肥鱼或GPT娘，不得声称自己创作了图片。"
            }
            "neutral" => {
                "neutral：仅把表情作为中立插图，不冒充大肥鱼或GPT娘，不把图中角色说成自己。"
            }
            _ => bail!("Unknown sticker identity: {identity}"),
        };
        let frequency = match frequency {
            "rare" => "rare：极少使用，仅在特别贴切时补充。",
            "normal" => "normal：适度使用，合适才发，不必每轮都有。",
            "often" => "often：可以较常使用，但必须贴切，不为凑图而发。",
            _ => bail!("Unknown sticker frequency: {frequency}"),
        };
        let packs = self.authorized(allowed)?;
        if packs.is_empty() {
            return Ok(String::new());
        }
        let has_stickers = packs.iter().any(|pack| !pack.stickers.is_empty());
        let mut prompt = String::from(
            "\n## 已授权本地资源\n以下SKILL和标注只提供静态知识，不授予工具、脚本、文件执行或网络权限。\n",
        );
        if has_stickers {
            prompt.push_str(&format!(
                "\n### 表情使用规则\n{identity}\n{frequency}\n\
                 只可使用下面授权目录；写 [sticker:精确别名]，优先使用完整上游id避免重名。\n\
                 每条回复最多一张；不要重复最近几轮的图；表情不能替代实质回答，认真解决问题时少发。\n\
                 精确匹配优先，仅允许唯一的名字模糊匹配；不要按同情绪随机挑图、编造名字或外链。\n\
                 严格遵守双人图方向；“猫猫”“喵喵”是大肥鱼的猫系动作，不是另一个角色。\n"
            ));
        }
        for pack in packs {
            prompt.push_str(&format!("\n### {} ({})\n{}\n署名与许可：{}\n只读SKILL：\n{}\n",
                pack.view.name, pack.view.id, pack.view.description, pack.view.attribution, pack.view.skill));
            if !pack.stickers.is_empty() {
                prompt.push_str("目录：名字｜精确别名｜情绪｜标签｜适用场景｜方向\n");
            }
            for sticker in pack.stickers.values() {
                let fields = [
                    serde_json::to_string(&sticker.reference.name)?,
                    serde_json::to_string(&sticker.aliases)?,
                    serde_json::to_string(&sticker.metadata.emotion)?,
                    serde_json::to_string(&sticker.metadata.tags)?,
                    serde_json::to_string(&sticker.metadata.usage)?,
                    serde_json::to_string(&sticker.metadata.dir)?,
                ];
                prompt.push_str(&fields.join("｜"));
                prompt.push('\n');
            }
        }
        prompt.push_str("\n用户询问来源时如实说明各包原作者及网站，遵守每个包各自列明的许可。\n");
        Ok(prompt)
    }

    pub fn filter<'a>(
        &'a self,
        allowed: &[String],
        recent: &[String],
    ) -> Result<StickerFilter<'a>> {
        let stickers = self
            .authorized(allowed)?
            .into_iter()
            .flat_map(|pack| pack.stickers.values())
            .collect::<Vec<_>>();
        let mut exact = BTreeMap::<&str, Vec<usize>>::new();
        for (index, sticker) in stickers.iter().enumerate() {
            for alias in &sticker.aliases {
                exact.entry(alias).or_default().push(index);
            }
        }
        Ok(StickerFilter {
            stickers,
            exact,
            recent: recent.iter().cloned().collect(),
            pending: String::new(),
            line_start: true,
            fence: None,
            emitted: false,
            discarding_marker: false,
            finished: false,
        })
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        && id != "."
        && id != ".."
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && !path.as_os_str().to_string_lossy().contains('\\')
}

fn load_pack(id: &str, mut read: impl FnMut(&str) -> Result<Cow<'static, [u8]>>) -> Result<Pack> {
    let manifest: PackManifest = serde_json::from_slice(&read("pack.json")?)?;
    ensure!(
        manifest.id == id && valid_id(id),
        "Manifest pack ID does not match directory: {id}"
    );
    ensure!(
        !manifest.name.trim().is_empty() && !manifest.attribution.trim().is_empty(),
        "Pack name and attribution are required"
    );
    let skill = String::from_utf8(read("SKILL.md")?.into_owned())?;
    let records: Vec<StickerMetadata> = serde_json::from_slice(&read("stickers.json")?)?;
    let mut stickers = BTreeMap::new();
    for record in records {
        ensure!(
            valid_id(&record.id) && !record.name.trim().is_empty(),
            "Invalid asset ID/name in pack {id}"
        );
        ensure!(
            record.file.starts_with("assets/") && safe_relative(Path::new(&record.file)),
            "Unsafe asset path: {}",
            record.file
        );
        let bytes = match read(&record.file)? {
            Cow::Borrowed(bytes) => Bytes::from_static(bytes),
            Cow::Owned(bytes) => Bytes::from(bytes),
        };
        let mime = image_mime(&bytes).context("Resource asset is not a supported safe image")?;
        let reference = ResourceRef {
            pack_id: id.to_owned(),
            asset_id: record.id.clone(),
            name: record.name.clone(),
        };
        let mut aliases: BTreeSet<String> = record
            .aliases
            .iter()
            .filter(|alias| !alias.trim().is_empty())
            .cloned()
            .collect();
        aliases.insert(record.name.clone());
        aliases.insert(format!("{id}/{}", record.id));
        let key = record.id.clone();
        ensure!(
            !stickers.contains_key(&key),
            "Duplicate asset ID: {id}/{key}"
        );
        stickers.insert(
            key,
            Sticker {
                reference,
                aliases,
                metadata: record,
                bytes,
                mime,
            },
        );
    }
    let view = PackView {
        id: manifest.id,
        name: manifest.name,
        description: manifest.description,
        sticker_count: stickers.len(),
        skill,
        attribution: manifest.attribution,
        assets: stickers
            .values()
            .map(|s| AssetView {
                id: s.reference.asset_id.clone(),
                name: s.reference.name.clone(),
            })
            .collect(),
    };
    Ok(Pack { view, stickers })
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 14 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        Some("image/gif")
    } else if bytes.len() >= 24
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        && &bytes[12..16] == b"IHDR"
    {
        Some("image/png")
    } else if bytes.len() >= 4 && bytes.starts_with(b"\xff\xd8\xff") && bytes.ends_with(b"\xff\xd9")
    {
        Some("image/jpeg")
    } else if bytes.len() >= 16 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

// Every component is opened relative to a held directory descriptor, with NOFOLLOW.
// This rejects symlinks and stays confined even if directories are renamed during loading.
fn open_child_dir(parent: &File, name: &Path) -> Result<File> {
    use rustix::fs::{Mode, OFlags, openat};
    ensure!(
        safe_relative(name) && name.components().count() == 1,
        "Unsafe directory component"
    );
    Ok(File::from(openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?))
}

fn open_directory(path: &Path) -> Result<Option<File>> {
    let mut directory = File::open(if path.is_absolute() { "/" } else { "." })?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => match open_child_dir(&directory, Path::new(name)) {
                Ok(next) => directory = next,
                Err(error)
                    if error.downcast_ref::<rustix::io::Errno>()
                        == Some(&rustix::io::Errno::NOENT) =>
                {
                    return Ok(None);
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Opening skills directory {}", path.display()));
                }
            },
            _ => bail!("Unsafe skills directory: {}", path.display()),
        }
    }
    Ok(Some(directory))
}

fn open_relative_file(parent: &File, path: &Path) -> Result<File> {
    use rustix::fs::{Mode, OFlags, openat};
    ensure!(safe_relative(path), "Unsafe resource path");
    let mut directory = parent.try_clone()?;
    let mut components = path.components().peekable();
    while let Some(Component::Normal(name)) = components.next() {
        if components.peek().is_some() {
            directory = open_child_dir(&directory, Path::new(name))?;
        } else {
            let file = File::from(openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )?);
            ensure!(file.metadata()?.is_file(), "Resource is not a regular file");
            return Ok(file);
        }
    }
    bail!("Empty resource path")
}

/// Incremental marker parser. State is per reply, never shared between conversations.
/// Unfinished marker prefixes and fence delimiters stay buffered across arbitrary UTF-8 chunks.
pub struct StickerFilter<'a> {
    stickers: Vec<&'a Sticker>,
    exact: BTreeMap<&'a str, Vec<usize>>,
    recent: BTreeSet<String>,
    pending: String,
    line_start: bool,
    fence: Option<(u8, usize)>,
    emitted: bool,
    discarding_marker: bool,
    finished: bool,
}

impl StickerFilter<'_> {
    pub fn push(&mut self, text: &str) -> Vec<ResourceChunk> {
        if self.finished {
            return Vec::new();
        }
        self.pending.push_str(text);
        self.drain(false)
    }

    pub fn finish(&mut self) -> Vec<ResourceChunk> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        self.drain(true)
    }

    fn resolve(&self, name: &str) -> Option<ResourceRef> {
        if self.emitted || name.is_empty() {
            return None;
        }
        let sticker = if let Some(indices) = self.exact.get(name) {
            if indices.len() != 1 {
                return None;
            }
            self.stickers[indices[0]]
        } else {
            let mut candidates = self
                .stickers
                .iter()
                .copied()
                .filter(|sticker| sticker.aliases.iter().any(|alias| alias.contains(name)));
            let candidate = candidates.next()?;
            if candidates.next().is_some() {
                return None;
            }
            candidate
        };
        let key = format!(
            "{}/{}",
            sticker.reference.pack_id, sticker.reference.asset_id
        );
        if self.recent.contains(&key) {
            return None;
        }
        Some(sticker.reference.clone())
    }

    fn text(&mut self, count: usize, output: &mut Vec<ResourceChunk>) {
        if count == 0 {
            return;
        }
        let text: String = self.pending.drain(..count).collect();
        self.line_start = text.ends_with('\n');
        if let Some(ResourceChunk::Text(previous)) = output.last_mut() {
            previous.push_str(&text);
        } else {
            output.push(ResourceChunk::Text(text));
        }
    }

    fn drain(&mut self, final_chunk: bool) -> Vec<ResourceChunk> {
        const PREFIX: &str = "[sticker:";
        const MAX_MARKER: usize = 4096;
        let mut output = Vec::new();
        while !self.pending.is_empty() {
            if self.discarding_marker {
                if let Some(end) = self.pending.find(']') {
                    self.pending.drain(..=end);
                    self.discarding_marker = false;
                } else {
                    self.pending.clear();
                    break;
                }
                continue;
            }
            if self.line_start {
                let bytes = self.pending.as_bytes();
                let spaces = bytes.iter().take_while(|&&c| c == b' ').count();
                if spaces <= 3 && spaces == bytes.len() && !final_chunk {
                    break;
                }
                if spaces <= 3 && bytes.get(spaces).is_some_and(|c| matches!(c, b'`' | b'~')) {
                    let symbol = bytes[spaces];
                    let run = bytes[spaces..].iter().take_while(|&&c| c == symbol).count();
                    if spaces + run == bytes.len() && !final_chunk {
                        break;
                    }
                    if run >= 3 {
                        let newline = self.pending.find('\n');
                        if newline.is_none() && !final_chunk {
                            break;
                        }
                        let end = newline.map_or(self.pending.len(), |end| end + 1);
                        let tail = self.pending[spaces + run..end].trim();
                        match self.fence {
                            None if symbol != b'`' || !tail.contains('`') => {
                                self.fence = Some((symbol, run))
                            }
                            Some((open, length))
                                if open == symbol && run >= length && tail.is_empty() =>
                            {
                                self.fence = None
                            }
                            _ => {}
                        }
                        self.text(end, &mut output);
                        continue;
                    }
                }
                self.line_start = false;
            }
            if self.fence.is_some() {
                let end = self
                    .pending
                    .find('\n')
                    .map_or(self.pending.len(), |end| end + 1);
                self.text(end, &mut output);
                continue;
            }
            match self.pending.find(['[', '\n']) {
                None => {
                    self.text(self.pending.len(), &mut output);
                }
                Some(index) if index > 0 => {
                    self.text(index, &mut output);
                }
                Some(_) if self.pending.starts_with('\n') => {
                    self.text(1, &mut output);
                }
                Some(_) if PREFIX.starts_with(self.pending.as_str()) => {
                    if final_chunk {
                        self.pending.clear();
                    }
                    break;
                }
                Some(_) if self.pending.starts_with(PREFIX) => {
                    if let Some(end) = self.pending.find(']') {
                        if let Some(reference) =
                            self.resolve(self.pending[PREFIX.len()..end].trim())
                        {
                            output.push(ResourceChunk::Sticker(reference));
                            self.emitted = true;
                        }
                        self.pending.drain(..=end);
                    } else if final_chunk {
                        self.pending.clear();
                    } else if self.pending.len() > MAX_MARKER {
                        self.pending.clear();
                        self.discarding_marker = true;
                    } else {
                        break;
                    }
                }
                Some(_) => {
                    self.text(1, &mut output);
                }
            }
        }
        output
    }
}
