//! Run every source (concurrently, one thread per site) and merge the items into the registry.

use std::collections::HashSet;
use std::path::Path;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json};

use crate::bail;
use crate::config::Config;
use crate::dedupe::{self, Query};
use crate::error::Result;
use crate::ledger;
use crate::paths;
use crate::registry::{self, Entry, NewEntry, new_entry, transition};
use crate::sources::{Item, SOURCES, Source};

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub added: Vec<String>,
    pub merged: Vec<Value>,
    pub duplicates: Vec<String>,
    pub seen: IndexMap<String, usize>,
    pub errors: Vec<String>,
    #[serde(skip)]
    pub new: Vec<Entry>,
}

/// `source` is a source name or `all` (all sources except `config.collect_skip`).
pub fn collect(config: &Config, source: &str, pages: u32) -> Result<Report> {
    collect_from(&paths::registry(), config, SOURCES, source, pages)
}

/// [`collect`] against an explicit registry file and source list.
/// Fetching happens outside the registry lock, one thread per source; only merging holds the lock.
/// A failing or empty source is recorded in `errors` and does not stop the others.
pub fn collect_from(
    registry_path: &Path,
    config: &Config,
    sources: &[Source],
    source: &str,
    pages: u32,
) -> Result<Report> {
    let known: HashSet<String> = registry::read_at(registry_path, |reg| Ok(reg.contests.keys().cloned().collect()))?;
    let selected: Vec<&Source> = if source == "all" {
        sources.iter().filter(|s| !config.collect_skip.iter().any(|skip| skip == s.name)).collect()
    } else {
        match sources.iter().find(|s| s.name == source) {
            Some(s) => vec![s],
            None => {
                let names: Vec<&str> = sources.iter().map(|s| s.name).collect();
                bail!("알 수 없는 출처: {source} (가능: {}, all)", names.join(", "));
            }
        }
    };
    let results: Vec<(&str, Result<Vec<Item>, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = selected
            .iter()
            .map(|s| {
                let known = &known;
                (s.name, scope.spawn(move || (s.collect)(pages, known)))
            })
            .collect();
        handles
            .into_iter()
            .map(|(name, handle)| {
                let result = match handle.join() {
                    Ok(Ok(rows)) => Ok(rows),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(panic) => Err(panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "panic".to_string())),
                };
                (name, result)
            })
            .collect()
    });

    let mut report = Report::default();
    let mut items: Vec<Item> = Vec::new();
    for (name, result) in results {
        match result {
            Ok(rows) => {
                report.seen.insert(name.to_string(), rows.len());
                if rows.is_empty() {
                    report
                        .errors
                        .push(format!("{name}: 목록 0건(열린 공모전이 정말 없거나 페이지 구조가 바뀌었을 수 있음)"));
                }
                items.extend(rows);
            }
            Err(err) => report.errors.push(format!("{name}: {err}")),
        }
    }

    // The ledger sits beside the registry (`data/submissions.json`).
    let submitted = ledger::load_at(&registry_path.with_file_name("submissions.json"))?;
    registry::write_at(registry_path, |reg| {
        for item in &items {
            if let Some(existing) = reg.contests.get_mut(&item.entry_id) {
                if item.deadline.as_deref().is_some_and(|d| !d.is_empty())
                    && existing.deadline.as_deref().is_none_or(str::is_empty)
                {
                    existing.deadline = item.deadline.clone();
                }
                continue;
            }
            // Strict title match or URL key: merge into the existing entry instead of adding a second one.
            let same_query = Query {
                title: &item.title,
                url: &item.url,
                include_folders: false,
                exclude_id: None,
                strict: true,
                organizer: "",
                deadline: None,
                submitted: None,
            };
            let same = dedupe::find_matches(reg, config, &same_query);
            if let Some(first) = same.first() {
                let target_id = first["id"].as_str().unwrap_or_default().to_string();
                let target = reg.get_mut(&target_id)?;
                for url in std::iter::once(item.url.as_str()).chain(item.alt_url.as_deref()) {
                    if !url.is_empty() && url != target.url && !target.alt_urls.iter().any(|u| u == url) {
                        target.alt_urls.push(url.to_string());
                    }
                }
                report.merged.push(json!({"id": target_id, "from": item.entry_id}));
                continue;
            }
            let mut entry = new_entry(NewEntry {
                id: &item.entry_id,
                title: &item.title,
                url: &item.url,
                source: item.entry_id.split('-').next().unwrap_or_default(),
                organizer: &item.organizer,
                deadline: item.deadline.clone(),
                categories: item.categories.clone(),
                note: item.note.clone(),
            });
            if let Some(alt) = item.alt_url.as_deref().filter(|u| !u.is_empty()) {
                entry.alt_urls.push(alt.to_string());
            }
            // Folders, the shared list and the submission ledger (by title or URL): mark it a duplicate.
            let folder_query = Query { include_folders: true, submitted: Some(&submitted), ..same_query };
            let folder_hit =
                dedupe::find_matches(reg, config, &folder_query).into_iter().find(|m| m["kind"] != "registry");
            if let Some(hit) = folder_hit {
                let note = if hit["kind"] == "submitted" {
                    format!(
                        "이미 제출함: {} ({})",
                        hit["title"].as_str().unwrap_or_default(),
                        hit["submitted_at"].as_str().unwrap_or_default()
                    )
                } else {
                    format!("기존 폴더와 같음: {}", hit["id"].as_str().unwrap_or_default())
                };
                transition(&mut entry, "duplicate", "harness collect", &note)?;
                report.duplicates.push(entry.id.clone());
            } else {
                report.added.push(entry.id.clone());
            }
            reg.contests.insert(entry.id.clone(), entry);
        }
        report.new = report.added.iter().filter_map(|id| reg.contests.get(id).cloned()).collect();
        Ok(())
    })?;
    Ok(report)
}
