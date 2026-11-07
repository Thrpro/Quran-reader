//! Read-only access to the same QUL resources used by the released reader.
//! Text normalization is confined to search copies; drawing uses source strings.
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use unicode_normalization::UnicodeNormalization;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
// The largest original JSON resource is under 8 MiB. A hard read ceiling also
// handles a file growing between its metadata check and the read operation.
const MAX_JSON_BYTES: u64 = 12 * 1024 * 1024;
const MAX_WORD_ID: u32 = 83_668;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub id: u32,
    pub text: String,
    pub surah: u32,
    pub ayah: u32,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub line_number: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_centered: bool,
    pub surah_number: Option<u32>,
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub page_number: u32,
    pub lines: Vec<Line>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Surah {
    pub id: u32,
    pub name_simple: String,
    pub name_arabic: String,
    pub verses_count: u32,
}
pub use crate::app_core::SearchKind;
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(skip)]
    pub kind: SearchKind,
    pub key: String,
    pub surah: u32,
    pub ayah: u32,
    pub page_number: u32,
    pub text: String,
    pub normalized_text: String,
    #[serde(default)]
    pub expanded_text: String,
}
#[derive(Deserialize, Serialize)]
pub(crate) struct Study {
    pub(crate) lookup: BTreeMap<String, serde_json::Value>,
    pub(crate) groups: BTreeMap<String, Group>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Group {
    pub(crate) ayah_keys: Vec<u32>,
    pub(crate) text: String,
}
#[derive(Deserialize)]
struct TranslationEntry {
    text: String,
    #[serde(default)]
    footnotes: BTreeMap<String, String>,
}

// Neighboring ayahs usually share one chapter file. Keep only a few validated
// chapters instead of reopening and decoding it on every arrow key press.
const MAX_STUDY_CHAPTERS: usize = 6;
const MAX_STUDY_CACHE_BYTES: usize = 4 * 1024 * 1024;
struct CachedStudy {
    source: String,
    surah: u32,
    study: Arc<Study>,
    bytes: usize,
}
#[derive(Default)]
struct StudyCache {
    chapters: VecDeque<CachedStudy>,
    bytes: usize,
}
impl StudyCache {
    fn get(&mut self, source: &str, surah: u32) -> Option<Arc<Study>> {
        let index = self
            .chapters
            .iter()
            .position(|entry| entry.source == source && entry.surah == surah)?;
        let entry = self.chapters.remove(index)?;
        let study = entry.study.clone();
        self.chapters.push_back(entry);
        Some(study)
    }
    fn insert(&mut self, source: &str, surah: u32, study: Arc<Study>) {
        // Account for source text and map/reference storage conservatively.
        // A chapter larger than the entire budget is used without caching.
        let bytes = study_bytes(&study);
        if bytes > MAX_STUDY_CACHE_BYTES {
            return;
        }
        while self.chapters.len() >= MAX_STUDY_CHAPTERS
            || self.bytes.saturating_add(bytes) > MAX_STUDY_CACHE_BYTES
        {
            if let Some(entry) = self.chapters.pop_front() {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.chapters.push_back(CachedStudy {
            source: source.to_owned(),
            surah,
            study,
            bytes,
        });
    }
}
fn study_bytes(study: &Study) -> usize {
    study
        .groups
        .iter()
        .map(|(key, group)| {
            key.len()
                .saturating_add(group.text.capacity())
                .saturating_add(group.ayah_keys.capacity() * std::mem::size_of::<u32>())
                .saturating_add(192)
        })
        .chain(study.lookup.iter().map(|(key, value)| {
            key.len()
                .saturating_add(value.as_str().map_or(32, str::len))
                .saturating_add(192)
        }))
        .fold(0usize, usize::saturating_add)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationRecord {
    key: String,
    surah: u32,
    ayah: u32,
    page_number: u32,
    #[serde(rename = "text", deserialize_with = "discard_source_text")]
    _text: (),
    #[serde(rename = "normalizedText", deserialize_with = "discard_index_text")]
    _normalized_text: (),
    #[serde(
        default,
        rename = "expandedText",
        deserialize_with = "discard_index_text"
    )]
    _expanded_text: (),
}
fn discard_source_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<(), D::Error> {
    discard_bounded_text(deserializer, true)
}
fn discard_index_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<(), D::Error> {
    discard_bounded_text(deserializer, false)
}
fn discard_bounded_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
    required: bool,
) -> std::result::Result<(), D::Error> {
    struct TextBound(bool);
    impl serde::de::Visitor<'_> for TextBound {
        type Value = ();
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded original search string")
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<(), E> {
            if value.len() > 32_768 || (self.0 && value.is_empty()) {
                return Err(E::custom("Invalid Quran search string"));
            }
            Ok(())
        }
    }
    // Validate the same string types/limits as the complete text index, then
    // discard them. Unescaped source strings borrow the JSON input here.
    deserializer.deserialize_str(TextBound(required))
}
fn navigation_from_records(
    records: &[NavigationRecord],
    surahs: &[Surah],
) -> Result<Vec<SearchHit>> {
    if records.len() != 6236 {
        return Err("Incomplete Quran navigation index".into());
    }
    let mut expected = (1, 1);
    let mut last_page = 1;
    let mut starts = Vec::with_capacity(114);
    for record in records {
        if !valid_reference(surahs, record.surah, record.ayah)
            || (record.surah, record.ayah) != expected
            || record.key != format!("{}:{}", record.surah, record.ayah)
            || !(last_page..=604).contains(&record.page_number)
        {
            return Err("Invalid or unordered Quran navigation record".into());
        }
        if record.ayah == 1 {
            starts.push(SearchHit {
                key: record.key.clone(),
                kind: SearchKind::Surah,
                surah: record.surah,
                ayah: 1,
                page_number: record.page_number,
                text: String::new(),
                normalized_text: String::new(),
                expanded_text: String::new(),
            });
        }
        last_page = record.page_number;
        expected = if record.ayah == surahs[record.surah as usize - 1].verses_count {
            (record.surah + 1, 1)
        } else {
            (record.surah, record.ayah + 1)
        };
    }
    if starts.len() != 114 || expected != (115, 1) {
        return Err("Incomplete Quran chapter navigation".into());
    }
    Ok(starts)
}

pub struct Resources {
    pub root: PathBuf,
    pub surahs: Vec<Surah>,
    pages: VecDeque<(u32, Arc<Page>)>,
    search: Option<Vec<SearchHit>>,
    navigation: Option<Vec<SearchHit>>,
    surah_search_names: Vec<String>,
    sections: RefCell<[Option<Vec<SearchHit>>; 3]>,
    studies: RefCell<StudyCache>,
    study_library: Option<Arc<crate::tafsir_library::Library>>,
}
impl Resources {
    pub fn open(root: PathBuf) -> Result<Self> {
        let surahs: Vec<Surah> = read_json(&root.join("data/qul/processed/surahs.json"))?;
        validate_surahs(&surahs)?;
        let surah_search_names = surahs
            .iter()
            .map(|surah| {
                normalize(&format!(
                    "{} {} {}",
                    surah.id, surah.name_simple, surah.name_arabic
                ))
            })
            .collect();
        Ok(Self {
            root,
            surahs,
            pages: VecDeque::new(),
            search: None,
            navigation: None,
            surah_search_names,
            sections: RefCell::new([None, None, None]),
            studies: RefCell::new(StudyCache::default()),
            study_library: None,
        })
    }
    pub fn page(&mut self, number: u32) -> Result<Arc<Page>> {
        if !(1..=604).contains(&number) {
            return Err("Invalid page number".into());
        }
        if let Some(index) = self.pages.iter().position(|(n, _)| *n == number) {
            let entry = self.pages.remove(index).unwrap();
            let page = entry.1.clone();
            self.pages.push_back(entry);
            return Ok(page);
        }
        let page: Page = read_json(
            &self
                .root
                .join(format!("experimental/digital-khatt/pages/p{number}.json")),
        )?;
        validate_page(&page, number, &self.surahs)?;
        let page = Arc::new(page);
        self.pages.push_back((number, page.clone()));
        if self.pages.len() > 12 {
            self.pages.pop_front();
        }
        Ok(page)
    }
    pub fn index(&mut self) -> Result<&[SearchHit]> {
        if self.search.is_none() {
            let index: Vec<SearchHit> = read_json(
                &self
                    .root
                    .join("experimental/digital-khatt/ayah-search-index.json"),
            )?;
            validate_index(&index, &self.surahs)?;
            self.search = Some(index);
        }
        Ok(self.search.as_ref().unwrap())
    }
    pub fn search(&mut self, query: &str) -> Result<Vec<SearchHit>> {
        if query.len() > 2048 {
            return Err("Search query is too long".into());
        }
        let query = normalize(query);
        if query.is_empty() {
            return Ok(vec![]);
        }
        let reference = query.split_once(':').and_then(|(s, a)| {
            Some((s.trim().parse::<u32>().ok()?, a.trim().parse::<u32>().ok()?))
        });
        Ok(self
            .index()?
            .iter()
            .filter(|hit| {
                reference.map_or_else(
                    || hit.normalized_text.contains(&query) || hit.expanded_text.contains(&query),
                    |(s, a)| hit.surah == s && hit.ayah == a,
                )
            })
            .take(80)
            .cloned()
            .collect())
    }
    fn navigation_starts(&mut self) -> Result<&[SearchHit]> {
        if self.navigation.is_none() {
            // The header-start map can precede a chapter's first ayah (An-Nisa
            // is one such page). Keep the exact first-ayah mapping already used
            // by Search, without retaining all Quran text just to browse names.
            let records = if let Some(index) = &self.search {
                index
                    .iter()
                    .map(|hit| NavigationRecord {
                        key: hit.key.clone(),
                        surah: hit.surah,
                        ayah: hit.ayah,
                        page_number: hit.page_number,
                        _text: (),
                        _normalized_text: (),
                        _expanded_text: (),
                    })
                    .collect()
            } else {
                read_json::<Vec<NavigationRecord>>(
                    &self
                        .root
                        .join("experimental/digital-khatt/ayah-search-index.json"),
                )?
            };
            self.navigation = Some(navigation_from_records(&records, &self.surahs)?);
        }
        Ok(self.navigation.as_ref().unwrap())
    }
    /// Browse surahs from original metadata, or preview a page without jumping.
    /// Matching normalization never modifies the text returned for reading.
    pub fn browse_search(&mut self, query: &str) -> Result<Vec<SearchHit>> {
        if query.len() > 2048 {
            return Err("Search query is too long".into());
        }
        if let Some(page_number) = jump_page(query) {
            return Ok(vec![SearchHit {
                key: page_number.to_string(),
                kind: SearchKind::Page,
                surah: 0,
                ayah: 0,
                page_number,
                text: String::new(),
                normalized_text: String::new(),
                expanded_text: String::new(),
            }]);
        }
        let normalized = normalize(query);
        let chapters: Vec<u32> = self
            .surahs
            .iter()
            .zip(&self.surah_search_names)
            .filter(|(_, name)| normalized.is_empty() || name.contains(&normalized))
            .map(|(s, _)| s.id)
            .collect();
        if !chapters.is_empty() {
            return Ok(self
                .navigation_starts()?
                .iter()
                .filter(|hit| chapters.contains(&hit.surah))
                .cloned()
                .collect());
        }
        self.search(query)
    }
    pub fn browse_surahs(&mut self, query: &str) -> Result<Vec<SearchHit>> {
        if query.len() > 2048 {
            return Err("Search query is too long".into());
        }
        let query = normalize(query);
        let number = query.parse::<u32>().ok();
        let ids: Vec<u32> = self
            .surahs
            .iter()
            .zip(&self.surah_search_names)
            .filter(|(s, name)| {
                if let Some(n) = number {
                    s.id == n
                } else {
                    query.is_empty() || name.contains(&query)
                }
            })
            .map(|(s, _)| s.id)
            .collect();
        Ok(self
            .navigation_starts()?
            .iter()
            .filter(|h| ids.contains(&h.surah))
            .cloned()
            .collect())
    }
    pub fn browse_sections(&self, kind: SearchKind, query: &str) -> Result<Vec<SearchHit>> {
        if query.len() > 2048 {
            return Err("Search query is too long".into());
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Start {
            page_number: u32,
            #[serde(default)]
            juz: u32,
            #[serde(default)]
            hizb: u32,
            #[serde(default)]
            rub: u32,
        }
        let (file, count, slot) = match kind {
            SearchKind::Juz => ("page-index.json", 604, 0),
            SearchKind::Hizb => ("hizb.json", 60, 1),
            SearchKind::Rub => ("rub.json", 240, 2),
            _ => return Err("Invalid browse section".into()),
        };
        let query = normalize(query);
        if let Some(cached) = &self.sections.borrow()[slot] {
            return Ok(cached
                .iter()
                .filter(|hit| query.is_empty() || hit.key == query)
                .cloned()
                .collect());
        }
        let starts: Vec<Start> =
            read_json_limit(&self.root.join("data/qul/processed").join(file), 128 * 1024)?;
        if starts.len() != count || starts.iter().any(|s| !(1..=604).contains(&s.page_number)) {
            return Err("Invalid original browse metadata".into());
        }
        let mut previous = 0;
        let mut previous_page = 1;
        let mut result = Vec::new();
        for (i, s) in starts.iter().enumerate() {
            if s.page_number < previous_page {
                return Err("Unordered original browse pages".into());
            }
            let n = match kind {
                SearchKind::Juz => s.juz,
                SearchKind::Hizb => s.hizb,
                _ => s.rub,
            };
            if kind == SearchKind::Juz {
                if s.page_number != i as u32 + 1
                    || !(1..=30).contains(&n)
                    || n < previous
                    || n > previous + 1
                {
                    return Err("Invalid original juz order".into());
                }
            } else if n != i as u32 + 1 {
                return Err("Invalid original section order".into());
            }
            if n != previous {
                result.push(SearchHit {
                    kind,
                    key: n.to_string(),
                    surah: 0,
                    ayah: 0,
                    page_number: s.page_number,
                    text: String::new(),
                    normalized_text: String::new(),
                    expanded_text: String::new(),
                });
            }
            previous = n;
            previous_page = s.page_number;
        }
        if kind == SearchKind::Juz && previous != 30 {
            return Err("Incomplete original juz order".into());
        }
        let matches = result
            .iter()
            .filter(|hit| query.is_empty() || hit.key == query)
            .cloned()
            .collect();
        self.sections.borrow_mut()[slot] = Some(result);
        Ok(matches)
    }
    pub fn study(&self, source: &str, surah: u32, ayah: u32) -> Result<String> {
        if let Some(index) = crate::tafsir_library::SOURCES
            .iter()
            .position(|item| item.id == source)
        {
            if !valid_reference(&self.surahs, surah, ayah) {
                return Err("Invalid study reference".into());
            }
            return self
                .study_library
                .as_ref()
                .ok_or("Optional tafsir library is not open")?
                .read_ayah(index, surah, ayah)
                .map(|text| text.unwrap_or_default());
        }
        if !["mukhtasar", "saadi", "sahih"].contains(&source)
            || !valid_reference(&self.surahs, surah, ayah)
        {
            return Err("Invalid study reference".into());
        }
        let cached = self.studies.borrow_mut().get(source, surah);
        let study = if let Some(study) = cached {
            study
        } else {
            let study = read_study(&self.root, source, surah)?;
            validate_study(&study, surah, &self.surahs)?;
            let study = Arc::new(study);
            self.studies
                .borrow_mut()
                .insert(source, surah, study.clone());
            study
        };
        Ok(study_text(&study, source == "saadi", ayah))
    }
    pub fn set_study_library(&mut self, library: Arc<crate::tafsir_library::Library>) {
        self.study_library = Some(library);
    }

    /// Original source strings in page order, for accessible text or copying.
    /// No search normalization or contextual glyph shaping enters this path.
    pub fn page_text(&mut self, number: u32) -> Result<String> {
        Ok(self
            .page(number)?
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\r\n"))
    }
}
pub(crate) fn study_text(study: &Study, saadi: bool, ayah: u32) -> String {
    let group_key = study.lookup.get(&ayah.to_string()).map(|v| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    });
    let group = group_key.and_then(|key| study.groups.get(&key));
    let Some(group) = group.filter(|g| g.ayah_keys.contains(&ayah)) else {
        return String::new();
    };
    let (selected, section_range, marked) = if saadi {
        section_details(&group.text, ayah)
    } else {
        (group.text.clone(), None, false)
    };
    // Author-marked Saadi ranges retain their original selection. Other shared
    // groups name only their explicit references: an interval must never imply
    // commentary for ayahs absent from a noncontiguous publisher mapping.
    let heading = if let Some((first, last)) = section_range {
        format!("{first}–{last}\r\n\r\n")
    } else if !marked && group.ayah_keys.len() > 1 {
        if group
            .ayah_keys
            .windows(2)
            .all(|pair| pair[1] == pair[0] + 1)
        {
            format!(
                "{}–{}\r\n\r\n",
                group.ayah_keys.first().unwrap(),
                group.ayah_keys.last().unwrap()
            )
        } else {
            format!(
                "{}\r\n\r\n",
                group
                    .ayah_keys
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    } else {
        String::new()
    };
    heading + &plain_text(&selected)
}
fn read_study(root: &Path, source: &str, surah: u32) -> Result<Study> {
    let path = root.join(format!("data/study/{source}/s{surah}.json"));
    if source != "sahih" {
        return read_json(&path);
    }
    // The released English resources are a translation-entry map, whereas
    // Arabic tafsir chapters use lookup/groups. Preserve both source schemas.
    let entries: BTreeMap<String, TranslationEntry> = read_json(&path)?;
    if entries.len() > 286 {
        return Err("Oversized translation chapter".into());
    }
    let mut study = Study {
        lookup: BTreeMap::new(),
        groups: BTreeMap::new(),
    };
    for (key, entry) in entries {
        let ayah = key.parse::<u32>().map_err(|_| "Invalid translation ayah")?;
        if !(1..=286).contains(&ayah)
            || entry.text.len() > 256 * 1024
            || entry.footnotes.len() > 128
            || entry
                .footnotes
                .iter()
                .any(|(id, text)| id.is_empty() || id.len() > 64 || text.len() > 256 * 1024)
        {
            return Err("Invalid translation entry".into());
        }
        let mut text = entry.text;
        for (i, footnote) in entry.footnotes.values().enumerate() {
            text.push_str(&format!("\r\n\r\n{}. {}", i + 1, footnote));
        }
        study
            .lookup
            .insert(key.clone(), serde_json::Value::String(key.clone()));
        study.groups.insert(
            key,
            Group {
                ayah_keys: vec![ayah],
                text,
            },
        );
    }
    Ok(study)
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    read_json_limit(path, MAX_JSON_BYTES)
}
pub fn read_json_limit<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    if limit > MAX_JSON_BYTES {
        return Err("JSON read limit exceeds the resource ceiling".into());
    }
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("Resource is not a bounded regular JSON file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("JSON resource exceeds its size limit".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) fn valid_reference(surahs: &[Surah], surah: u32, ayah: u32) -> bool {
    surah
        .checked_sub(1)
        .and_then(|i| surahs.get(i as usize))
        .is_some_and(|chapter| chapter.id == surah && (1..=chapter.verses_count).contains(&ayah))
}
fn validate_study(study: &Study, surah: u32, surahs: &[Surah]) -> Result<()> {
    let count = surahs[surah as usize - 1].verses_count as usize;
    if study.groups.len() > count || study.lookup.len() > count {
        return Err("Oversized study chapter index".into());
    }
    for (key, group) in &study.groups {
        if key.is_empty()
            || key.len() > 64
            || group.text.len() > MAX_JSON_BYTES as usize
            || group.ayah_keys.is_empty()
            || group.ayah_keys.len() > count
            || group.ayah_keys.windows(2).any(|pair| pair[0] >= pair[1])
            || group
                .ayah_keys
                .iter()
                .any(|ayah| !valid_reference(surahs, surah, *ayah))
        {
            return Err("Invalid study ayah group".into());
        }
    }
    for (key, value) in &study.lookup {
        let ayah = key.parse::<u32>().ok();
        let group_key = value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_u64().map(|number| number.to_string()));
        if !ayah.is_some_and(|ayah| valid_reference(surahs, surah, ayah))
            || !group_key
                .and_then(|key| study.groups.get(&key))
                .is_some_and(|group| group.ayah_keys.contains(&ayah.unwrap()))
        {
            return Err("Invalid study chapter lookup".into());
        }
    }
    Ok(())
}
fn validate_surahs(surahs: &[Surah]) -> Result<()> {
    if surahs.len() != 114
        || surahs.iter().enumerate().any(|(i, chapter)| {
            chapter.id != i as u32 + 1
                || !(1..=286).contains(&chapter.verses_count)
                || chapter.name_simple.is_empty()
                || chapter.name_arabic.is_empty()
                || chapter.name_simple.len() > 512
                || chapter.name_arabic.len() > 512
        })
        || surahs.iter().map(|s| s.verses_count as u64).sum::<u64>() != 6236
    {
        return Err("Invalid Quran chapter index".into());
    }
    Ok(())
}
fn validate_page(page: &Page, number: u32, surahs: &[Surah]) -> Result<()> {
    let mut ids = BTreeSet::new();
    if page.page_number != number
        || page.lines.is_empty()
        || page.lines.len() > 15
        || page
            .lines
            .windows(2)
            .any(|pair| pair[0].line_number >= pair[1].line_number)
    {
        return Err("Invalid Quran page layout".into());
    }
    for line in &page.lines {
        if !(1..=15).contains(&line.line_number)
            || !["ayah", "surah_name", "basmallah"].contains(&line.kind.as_str())
            || line.text.len() > 16_384
            || line.words.len() > 64
            || (line.kind != "ayah" && !line.words.is_empty())
            || (line.kind == "surah_name"
                && !line.surah_number.is_some_and(|s| (1..=114).contains(&s)))
        {
            return Err("Invalid Quran source line".into());
        }
        for word in &line.words {
            if !(1..=MAX_WORD_ID).contains(&word.id)
                || !ids.insert(word.id)
                || !valid_reference(surahs, word.surah, word.ayah)
                || word.text.is_empty()
                || word.text.len() > 1024
            {
                return Err("Invalid Quran source word".into());
            }
        }
        if line.kind == "ayah"
            && (line.words.is_empty()
                || line
                    .words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    != line.text)
        {
            return Err("Quran words do not match the original line".into());
        }
    }
    Ok(())
}
fn validate_index(index: &[SearchHit], surahs: &[Surah]) -> Result<()> {
    let mut expected_surah = 1;
    let mut expected_ayah = 1;
    let mut last_page = 1;
    if index.len() != 6236 {
        return Err("Incomplete Quran search index".into());
    }
    for hit in index {
        if !valid_reference(surahs, hit.surah, hit.ayah)
            || (hit.surah, hit.ayah) != (expected_surah, expected_ayah)
            || hit.key != format!("{}:{}", hit.surah, hit.ayah)
            || !(last_page..=604).contains(&hit.page_number)
            || hit.text.is_empty()
            || hit.text.len() > 32_768
            || hit.normalized_text.len() > 32_768
            || hit.expanded_text.len() > 32_768
        {
            return Err("Invalid or unordered Quran search record".into());
        }
        last_page = hit.page_number;
        if hit.ayah == surahs[hit.surah as usize - 1].verses_count {
            expected_surah += 1;
            expected_ayah = 1;
        } else {
            expected_ayah += 1;
        }
    }
    Ok(())
}
pub fn normalize(text: &str) -> String {
    text.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c) && *c != '\u{0640}')
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            'ى' => 'ي',
            'ؤ' => 'و',
            'ئ' => 'ي',
            '٠'..='٩' => char::from_u32(c as u32 - '٠' as u32 + '0' as u32).unwrap(),
            '۰'..='۹' => char::from_u32(c as u32 - '۰' as u32 + '0' as u32).unwrap(),
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}
pub fn jump_page(text: &str) -> Option<u32> {
    normalize(text)
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=604).contains(n))
}
pub fn spread(number: u32) -> (u32, Option<u32>) {
    crate::app_core::spread(number)
}
/// HTML is never executed in the native reader. Source numeric references alone
/// select sections; quoted Quran words and commentary remain unchanged.
#[cfg(test)]
fn marked_section(html: &str, ayah: u32) -> String {
    section_details(html, ayah).0
}
fn section_details(html: &str, ayah: u32) -> (String, Option<(u32, u32)>, bool) {
    let mut sections = Vec::new();
    let mut selected_range = None;
    let mut offset = 0;
    let mut has_numeric_marker = false;
    while let Some(start) = html[offset..].find("<div") {
        let start = offset + start;
        let Some(close) = html[start..].find("</div>") else {
            break;
        };
        let end = start + close + 6;
        let block = &html[start..end];
        let marker = block.find("<span").and_then(|p| {
            let opening = p + block[p..].find('>')? + 1;
            let closing = opening + block[opening..].find("</span>")?;
            let token = plain_text(&block[opening..closing]);
            numeric_marker(&token).map(|numbers| (numbers, opening, closing))
        });
        if let Some((numbers, opening, closing)) = marker {
            has_numeric_marker = true;
            if numbers.contains(&ayah) {
                if numbers.len() > 1 {
                    selected_range = Some((*numbers.first().unwrap(), *numbers.last().unwrap()));
                }
                sections.push(format!("{}{}", &block[..opening], &block[closing..]));
            }
        }
        offset = end;
    }
    if sections.is_empty() {
        // Unmarked shared source text remains labeled as shared by the caller.
        if has_numeric_marker {
            (String::new(), None, true)
        } else {
            (html.to_owned(), None, false)
        }
    } else {
        (sections.join("\r\n"), selected_range, true)
    }
}
fn numeric_marker(text: &str) -> Option<Vec<u32>> {
    let text = normalize(&text.replace('ـ', "-"));
    let text = text.strip_prefix('{')?.strip_suffix('}')?;
    let parts: Vec<&str> = text.split(['ـ', '–', '—', '-']).collect();
    // Tatweel is retained here as a range separator, before search normalization.
    let parts: Vec<&str> = if parts.len() == 1 && text.contains(' ') {
        text.split_whitespace().collect()
    } else {
        parts
    };
    let nums = parts
        .iter()
        .map(|s| s.trim().parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    if nums.is_empty() || nums.iter().any(|n| !(1..=286).contains(n)) {
        return None;
    }
    if nums.len() == 2 && nums[1] >= nums[0] {
        Some((nums[0]..=nums[1]).collect())
    } else {
        Some(nums)
    }
}
pub fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            for c in chars.by_ref() {
                if c == '>' {
                    break;
                }
                tag.push(c);
            }
            if tag.starts_with("/div") || tag.starts_with("/p") || tag.starts_with("br") {
                out.push_str("\r\n\r\n");
            }
        } else {
            out.push(c);
        }
    }
    html_escape::decode_html_entities(&out).trim().to_owned()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub page: u32,
    pub arabic: bool,
    pub theme: u32,
    pub speed: f64,
    pub remember_speed: bool,
    pub volume: f64,
    pub contextual: bool,
    pub continuous_audio: bool,
    pub recording_clock_version: u32,
    pub offline_limit_gib: u32,
    pub show_clock: bool,
    pub clock_24: bool,
    pub study_text_size: i32,
    pub follow_tafsir: bool,
    pub search_key_t: bool,
    pub repeat_plays: u32,
    pub language_chosen: bool,
    pub other_study_languages: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            page: 1,
            arabic: true,
            theme: 0,
            speed: 1.0,
            remember_speed: false,
            volume: 0.8,
            contextual: false,
            continuous_audio: true,
            recording_clock_version: 0,
            offline_limit_gib: 1,
            show_clock: true,
            clock_24: false,
            study_text_size: 18,
            follow_tafsir: false,
            search_key_t: false,
            repeat_plays: 1,
            language_chosen: false,
            other_study_languages: false,
        }
    }
}
impl Preferences {
    /// Upgrade the old clip default once. A later explicit clip choice stays intact.
    pub fn upgrade_recording_clock(&mut self) -> bool {
        if self.recording_clock_version != 0 {
            return false;
        }
        self.continuous_audio = true;
        self.recording_clock_version = 1;
        true
    }
}
#[cfg(test)]
mod recording_preference_tests {
    use super::Preferences;
    #[test]
    fn new_profiles_prefer_chapter_audio_and_existing_choice_is_preserved() {
        assert!(
            serde_json::from_str::<Preferences>("{}")
                .unwrap()
                .continuous_audio
        );
        let prefs: Preferences = serde_json::from_str(r#"{"continuous_audio":false}"#).unwrap();
        assert!(!prefs.continuous_audio);
        let saved = serde_json::to_string(&prefs).unwrap();
        assert!(
            !serde_json::from_str::<Preferences>(&saved)
                .unwrap()
                .continuous_audio
        );
    }
}
impl Preferences {
    pub fn sanitize(&mut self) {
        if ![0, 1, 2, 3, 5, 10].contains(&self.repeat_plays) {
            self.repeat_plays = 1;
        }
        self.page = self.page.clamp(1, 604);
        self.theme = self.theme.min(3);
        self.study_text_size = self.study_text_size.clamp(14, 28);
        self.speed = if self.speed.is_finite() {
            self.speed.clamp(0.5, 2.0)
        } else {
            1.0
        };
        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            0.8
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_bounds() {
        assert_eq!(spread(604), (603, Some(604)));
        assert_eq!(spread(45), (45, Some(46)));
        assert_eq!(jump_page("٤٥"), Some(45));
        assert_eq!(jump_page("605"), None);
    }
    #[test]
    fn study_preference_is_backward_compatible_and_bounded() {
        let mut old: Preferences = serde_json::from_str("{\"page\":45}").unwrap();
        assert_eq!(old.study_text_size, 18);
        old.study_text_size = 24;
        let saved = serde_json::to_vec(&old).unwrap();
        assert_eq!(
            serde_json::from_slice::<Preferences>(&saved)
                .unwrap()
                .study_text_size,
            24
        );
        old.study_text_size = i32::MAX;
        old.sanitize();
        assert_eq!(old.study_text_size, 28);
        old.study_text_size = i32::MIN;
        old.sanitize();
        assert_eq!(old.study_text_size, 14);
    }
    #[test]
    fn continuous_recording_default_preserves_explicit_clip_preference() {
        assert!(Preferences::default().continuous_audio);
        assert!(
            serde_json::from_str::<Preferences>("{}")
                .unwrap()
                .continuous_audio
        );
        assert!(
            !serde_json::from_str::<Preferences>(r#"{"continuous_audio":false}"#)
                .unwrap()
                .continuous_audio
        );
    }
    #[test]
    fn search_keeps_source() {
        let source = "ٱلرَّحْمَٰنِ";
        assert_eq!(normalize(source), "الرحمن");
        assert_eq!(source, "ٱلرَّحْمَٰنِ");
    }
    #[test]
    fn browse_search_filters_original_chapters_and_previews_without_changing_source() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public-qul");
        let mut resources = Resources::open(root).unwrap();
        let chapters = resources.browse_search("").unwrap();
        assert_eq!(chapters.len(), 114);
        let matches = resources.browse_search("baqar").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].surah, 2);
        assert_eq!(matches[0].kind, SearchKind::Surah);
        assert!(matches[0].text.is_empty());
        assert!(!resources.search("2:1").unwrap()[0].text.is_empty());
        assert_eq!(resources.browse_search("البقرة").unwrap()[0].surah, 2);
        let page = resources.browse_search("٢٥٥").unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!((page[0].page_number, page[0].surah), (255, 0));
        assert_eq!(resources.browse_search("46:21").unwrap()[0].key, "46:21");
        assert!(resources.browse_search("x".repeat(2049).as_str()).is_err());
    }
    #[test]
    fn section_browse_uses_original_starts_and_surah_numbers_remain_distinct_from_pages() {
        let mut r =
            Resources::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public-qul"))
                .unwrap();
        assert_eq!(r.browse_surahs("٧٨").unwrap()[0].surah, 78);
        assert_eq!(r.browse_search("٧٨").unwrap()[0].page_number, 78);
        for (kind, count) in [
            (SearchKind::Juz, 30),
            (SearchKind::Hizb, 60),
            (SearchKind::Rub, 240),
        ] {
            let all = r.browse_sections(kind, "").unwrap();
            assert_eq!(all.len(), count);
            assert_eq!(all[0].page_number, 1);
            assert!(all.windows(2).all(|p| p[0].page_number <= p[1].page_number));
            assert_eq!(r.browse_sections(kind, "۲").unwrap()[0].key, "2");
        }
        assert_eq!(
            r.browse_sections(SearchKind::Juz, "30").unwrap()[0].page_number,
            582
        );
    }
    #[test]
    fn chapter_browse_retains_exact_ayah_starts_without_loading_full_text_index() {
        let mut r = original_resources();
        let original_root = r.root.clone();
        r.root = original_root.join("missing-navigation-fixture");
        assert!(r.browse_surahs("").is_err());
        assert!(r.navigation.is_none());
        r.root = original_root.clone();
        let chapters = r.browse_surahs("").unwrap();
        assert!(r.search.is_none());
        assert_eq!(chapters.len(), 114);
        // The authentic header can be on page 76 while An-Nisa's first ayah
        // starts on 77. A nominal chapter map must not change navigation.
        assert_eq!(chapters[3].page_number, 77);
        assert!(chapters.iter().all(|hit| hit.text.is_empty()
            && hit.normalized_text.is_empty()
            && hit.expanded_text.is_empty()));
        r.root = original_root.join("missing-cached-navigation-fixture");
        assert_eq!(r.browse_surahs("النساء").unwrap()[0].page_number, 77);
        assert_eq!(r.browse_search("naba").unwrap()[0].surah, 78);
        assert!(r.search.is_none());
        r.root = original_root;
        let full = r.index().unwrap();
        for (chapter, original) in chapters.iter().zip(full.iter().filter(|hit| hit.ayah == 1)) {
            assert_eq!(
                (&chapter.key, chapter.surah, chapter.page_number),
                (&original.key, original.surah, original.page_number)
            );
            assert!(!original.text.is_empty());
        }
    }
    #[test]
    fn section_filters_use_validated_cached_metadata_and_keep_input_bounds() {
        let mut r = original_resources();
        let original_root = r.root.clone();
        r.root = original_root.join("missing-sections-fixture");
        for kind in [SearchKind::Juz, SearchKind::Hizb, SearchKind::Rub] {
            assert!(r.browse_sections(kind, "").is_err());
        }
        r.root = original_root;
        let mut checked = Vec::new();
        for (kind, count) in [
            (SearchKind::Juz, 30),
            (SearchKind::Hizb, 60),
            (SearchKind::Rub, 240),
        ] {
            let all = r.browse_sections(kind, "").unwrap();
            assert_eq!(all.len(), count);
            checked.push((kind, all));
        }
        // A missing root simulates disk unavailability after immutable
        // metadata has been checked; subsequent filters need no IO.
        r.root = r.root.join("missing-cached-sections-fixture");
        for (kind, all) in checked {
            assert_eq!(
                r.browse_sections(kind, "۲").unwrap()[0].page_number,
                all[1].page_number
            );
            assert_eq!(r.browse_sections(kind, "").unwrap().len(), all.len());
            assert!(r.browse_sections(kind, &"x".repeat(2049)).is_err());
        }
        assert!(r.browse_surahs(&"x".repeat(2049)).is_err());
        assert!(r.browse_sections(SearchKind::Ayah, "").is_err());
    }
    #[test]
    fn lightweight_navigation_rejects_incomplete_or_unordered_records_and_bad_text() {
        let r = original_resources();
        let mut records: Vec<NavigationRecord> = read_json(
            &r.root
                .join("experimental/digital-khatt/ayah-search-index.json"),
        )
        .unwrap();
        assert_eq!(
            navigation_from_records(&records, &r.surahs).unwrap().len(),
            114
        );
        records[0].page_number = 0;
        assert!(navigation_from_records(&records, &r.surahs).is_err());
        records[0].page_number = 1;
        records.swap(0, 1);
        assert!(navigation_from_records(&records, &r.surahs).is_err());
        records.swap(0, 1);
        records.pop();
        assert!(navigation_from_records(&records, &r.surahs).is_err());
        for text in [
            serde_json::json!(""),
            serde_json::json!(5),
            serde_json::json!("x".repeat(32_769)),
        ] {
            let bad = serde_json::json!({"key":"1:1","surah":1,"ayah":1,"pageNumber":1,
                "text":text,"normalizedText":"valid"});
            assert!(serde_json::from_value::<NavigationRecord>(bad).is_err());
        }
        let bad = serde_json::json!({"key":"1:1","surah":1,"ayah":1,"pageNumber":1,
            "text":"authentic fixture","normalizedText":false});
        assert!(serde_json::from_value::<NavigationRecord>(bad).is_err());
    }
    #[test]
    fn section_selection() {
        let h = "<div><span>{21}</span>first</div><div><span>{22}</span>second</div><div><span>{24 ـ 25}</span>shared</div>";
        assert_eq!(plain_text(&marked_section(h, 22)), "second");
        assert_eq!(plain_text(&marked_section(h, 24)), "shared");
        assert!(marked_section(h, 23).is_empty());
        let unmarked = section_details("<div>shared explanation</div>", 24);
        assert_eq!(
            unmarked,
            ("<div>shared explanation</div>".into(), None, false)
        );
    }
    #[test]
    fn shared_labels_name_only_explicit_ayahs_and_preserve_marked_sections() {
        let chapter = |references: Vec<u32>, text: &str| Study {
            lookup: references
                .iter()
                .map(|ayah| (ayah.to_string(), serde_json::Value::String("1".into())))
                .collect(),
            groups: BTreeMap::from([(
                "1".into(),
                Group {
                    ayah_keys: references,
                    text: text.into(),
                },
            )]),
        };
        let disjoint = chapter(vec![2, 4], "Exact publisher notes.");
        assert_eq!(
            study_text(&disjoint, false, 2),
            "2, 4\r\n\r\nExact publisher notes."
        );
        assert_eq!(
            study_text(&disjoint, false, 4),
            "2, 4\r\n\r\nExact publisher notes."
        );
        assert!(study_text(&disjoint, false, 3).is_empty());
        let contiguous = chapter(vec![2, 3, 4], "Exact publisher notes.");
        assert_eq!(
            study_text(&contiguous, false, 3),
            "2–4\r\n\r\nExact publisher notes."
        );
        let marked = chapter(
            vec![2, 4],
            "<div><span>{2}</span>Second notes.</div><div><span>{4}</span>Fourth notes.</div>",
        );
        assert_eq!(study_text(&marked, true, 2), "Second notes.");
        assert_eq!(study_text(&marked, true, 4), "Fourth notes.");
        let marked_range = chapter(
            vec![2, 3, 4, 5],
            "<div><span>{2 ـ 4}</span>Marked range notes.</div><div><span>{5}</span>Fifth notes.</div>",
        );
        assert_eq!(
            study_text(&marked_range, true, 3),
            "2–4\r\n\r\nMarked range notes."
        );
    }
    #[test]
    fn preferences_are_bounded() {
        let mut p = Preferences {
            page: 999,
            theme: 9,
            speed: f64::NAN,
            volume: 3.0,
            ..Default::default()
        };
        p.sanitize();
        assert_eq!(p.page, 604);
        assert_eq!(p.speed, 1.0);
        assert_eq!(p.volume, 1.0);
    }
    fn original_resources() -> Resources {
        Resources::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public-qul")).unwrap()
    }
    #[test]
    fn original_pages_and_search_pass_structural_validation() {
        let mut resources = original_resources();
        for page in 1..=604 {
            assert_eq!(resources.page(page).unwrap().page_number, page);
        }
        assert_eq!(resources.index().unwrap().len(), 6236);
    }
    #[test]
    fn authentic_saadi_shared_range_is_labeled_without_splitting() {
        let resources = original_resources();
        let ayah24 = resources.study("saadi", 46, 24).unwrap();
        let ayah25 = resources.study("saadi", 46, 25).unwrap();
        let ayah23 = resources.study("saadi", 46, 23).unwrap();
        assert!(ayah24.starts_with("24–25\r\n\r\n"));
        assert_eq!(ayah24, ayah25);
        assert!(!ayah23.starts_with("24–25"));
        assert!(ayah24.contains("فأرسل الله"));
    }
    #[test]
    fn corrupt_references_and_duplicates_fail_without_indexing_panic() {
        let mut resources = original_resources();
        let mut chapters = resources.surahs.clone();
        chapters[0].id = 0;
        assert!(validate_surahs(&chapters).is_err());
        chapters[0].id = 1;
        chapters[0].verses_count = u32::MAX;
        assert!(validate_surahs(&chapters).is_err());
        let mut page = (*resources.page(45).unwrap()).clone();
        page.lines[0].words[0].surah = 115;
        assert!(validate_page(&page, 45, &resources.surahs).is_err());
        let mut page = (*resources.page(45).unwrap()).clone();
        page.lines[0].words[1].id = page.lines[0].words[0].id;
        assert!(validate_page(&page, 45, &resources.surahs).is_err());
        let mut index = resources.index().unwrap().to_vec();
        index[0].surah = u32::MAX;
        assert!(validate_index(&index, &resources.surahs).is_err());
        index[0].surah = 1;
        index.swap(0, 1);
        assert!(validate_index(&index, &resources.surahs).is_err());
        assert!(resources.study("../saadi", 46, 21).is_err());
        assert!(resources.study("saadi", 115, 1).is_err());
        assert!(resources.search(&"x".repeat(2049)).is_err());
    }
    #[test]
    fn oversized_json_is_rejected_before_deserialization() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "quran-native-bounded-json-{}-{nonce}.tmp",
            std::process::id()
        ));
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.set_len(MAX_JSON_BYTES + 1).unwrap();
        drop(file);
        let result = read_json::<serde_json::Value>(&path);
        fs::remove_file(path).unwrap();
        assert!(result.is_err());
    }
    #[test]
    fn all_bundled_study_chapters_pass_validation_including_english_schema() {
        let resources = original_resources();
        for source in ["mukhtasar", "saadi", "sahih"] {
            for surah in 1..=114 {
                let chapter = read_study(&resources.root, source, surah).unwrap();
                validate_study(&chapter, surah, &resources.surahs).unwrap();
                assert_eq!(
                    chapter.lookup.len(),
                    resources.surahs[surah as usize - 1].verses_count as usize
                );
            }
        }
        let translation = resources.study("sahih", 1, 1).unwrap();
        assert!(translation.starts_with("In the name of Allāh"));
        assert!(translation.contains("proper name belonging only"));
        assert!(!translation.contains("<sup"));
    }
    #[test]
    fn study_cache_is_lru_bounded_and_failed_reads_are_not_cached() {
        let mut resources = original_resources();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("native-study-cache-{}-{nonce}", std::process::id()));
        let chapters = root.join("data/study/mukhtasar");
        fs::create_dir_all(&chapters).unwrap();
        resources.root = root.clone();
        for surah in 1..=7 {
            fs::write(
                chapters.join(format!("s{surah}.json")),
                format!(
                    r#"{{"lookup":{{"1":"1"}},"groups":{{"1":{{"ayahKeys":[1],"text":"chapter {surah}"}}}}}}"#
                ),
            )
            .unwrap();
        }
        for surah in 1..=6 {
            assert_eq!(
                resources.study("mukhtasar", surah, 1).unwrap(),
                format!("chapter {surah}")
            );
        }
        fs::remove_file(chapters.join("s1.json")).unwrap();
        assert_eq!(resources.study("mukhtasar", 1, 1).unwrap(), "chapter 1");
        resources.study("mukhtasar", 7, 1).unwrap();
        fs::remove_file(chapters.join("s2.json")).unwrap();
        assert!(resources.study("mukhtasar", 2, 1).is_err());
        fs::write(
            chapters.join("s2.json"),
            r#"{"lookup":{"1":"1"},"groups":{"1":{"ayahKeys":[1],"text":"recovered"}}}"#,
        )
        .unwrap();
        assert_eq!(resources.study("mukhtasar", 2, 1).unwrap(), "recovered");
        let cache = resources.studies.borrow();
        assert_eq!(cache.chapters.len(), MAX_STUDY_CHAPTERS);
        assert!(cache.bytes <= MAX_STUDY_CACHE_BYTES);
        drop(cache);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn accessible_page_text_is_exact_source_and_corrupt_study_lookup_fails() {
        let mut resources = original_resources();
        let expected = resources
            .page(45)
            .unwrap()
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\r\n");
        assert_eq!(resources.page_text(45).unwrap(), expected);
        let mut chapter = read_study(&resources.root, "mukhtasar", 1).unwrap();
        chapter
            .lookup
            .insert("1".into(), serde_json::Value::String("missing".into()));
        assert!(validate_study(&chapter, 1, &resources.surahs).is_err());
    }
    #[test]
    fn study_cache_byte_budget_evicts_and_oversized_chapters_are_not_retained() {
        let mut cache = StudyCache::default();
        let chapter = |bytes: usize| {
            Arc::new(Study {
                lookup: BTreeMap::from([("1".into(), serde_json::Value::String("1".into()))]),
                groups: BTreeMap::from([(
                    "1".into(),
                    Group {
                        ayah_keys: vec![1],
                        text: "x".repeat(bytes),
                    },
                )]),
            })
        };
        cache.insert("saadi", 1, chapter(MAX_STUDY_CACHE_BYTES / 2));
        cache.insert("saadi", 2, chapter(MAX_STUDY_CACHE_BYTES / 2));
        assert_eq!(cache.chapters.len(), 1);
        assert!(cache.get("saadi", 1).is_none());
        cache.insert("saadi", 3, chapter(MAX_STUDY_CACHE_BYTES + 1));
        assert!(cache.get("saadi", 3).is_none());
        assert!(cache.bytes <= MAX_STUDY_CACHE_BYTES);
    }
}

#[cfg(test)]
mod recording_clock_upgrade_tests {
    use super::Preferences;
    #[test]
    fn upgrades_legacy_clip_default_once_and_preserves_later_choice() {
        let mut p: Preferences = serde_json::from_str(r#"{"continuous_audio":false}"#).unwrap();
        assert!(p.upgrade_recording_clock());
        assert!(p.continuous_audio);
        p.continuous_audio = false;
        let mut loaded: Preferences =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(!loaded.upgrade_recording_clock());
        assert!(!loaded.continuous_audio);
    }
}

#[cfg(test)]
mod study_follow_preferences_tests {
    #[test]
    fn following_is_opt_in_and_round_trips() {
        let mut prefs: super::Preferences = serde_json::from_str("{}").unwrap();
        assert!(!prefs.follow_tafsir);
        prefs.follow_tafsir = true;
        let saved = serde_json::to_string(&prefs).unwrap();
        assert!(
            serde_json::from_str::<super::Preferences>(&saved)
                .unwrap()
                .follow_tafsir
        );
    }
}

impl Preferences {
    pub fn begin_recitation(&mut self) -> bool {
        if !self.remember_speed && self.speed != 1. {
            self.speed = 1.;
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod speed_memory_tests {
    #[test]
    fn a_new_recitation_uses_normal_speed_unless_explicitly_remembered() {
        let mut p: super::Preferences = serde_json::from_str(r#"{"speed":1.5}"#).unwrap();
        assert!(!p.remember_speed);
        assert!(p.begin_recitation());
        assert_eq!(p.speed, 1.);
        p.speed = 1.5;
        p.remember_speed = true;
        assert!(!p.begin_recitation());
        assert_eq!(p.speed, 1.5);
        let saved = serde_json::to_string(&p).unwrap();
        assert!(
            serde_json::from_str::<super::Preferences>(&saved)
                .unwrap()
                .remember_speed
        );
    }
}
