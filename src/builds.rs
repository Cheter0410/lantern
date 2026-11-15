//! Builds: shapes and bullet points that come (or go) one click after the
//! other, as PowerPoint's animations set them up. LibreOffice puts only the
//! finished slide into the PDF, so Lantern makes a copy of the presentation
//! with one slide for each step – shapes not there yet are left out, bullet
//! points not there yet are made invisible (so nothing else moves) – and
//! GO goes through the steps before the next slide.
//!
//! The motion of an effect (fly in, zoom …) is not played: a build appears
//! with a short fade, or at once for PowerPoint's "Appear".

use std::collections::HashMap;
use std::io::{Read, Write};
use std::ops::Range;
use std::path::Path;

/// What an effect works on: a shape, or some of its paragraphs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub spid: String,
    /// Paragraphs `st..=end` (from 0) of the shape's text.
    pub paras: Option<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub target: Target,
    /// Entrance (else exit).
    pub entr: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Click {
    pub effects: Vec<Effect>,
    /// Seconds of the fade (0: at once).
    pub fade: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Builds {
    /// Effects that run by themselves when the slide comes.
    pub auto: Vec<Effect>,
    /// One per click.
    pub clicks: Vec<Click>,
}

fn named<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.tag_name().name() == name)
}

/// The builds of a slide (`p:timing`, main sequence).
pub fn parse(doc: &roxmltree::Document) -> Builds {
    let mut b = Builds::default();
    let Some(main) = doc
        .descendants()
        .find(|n| n.tag_name().name() == "cTn" && n.attribute("nodeType") == Some("mainSeq"))
    else {
        return b;
    };
    let Some(list) = named(main, "childTnLst") else {
        return b;
    };
    for group in list.children().filter(|n| n.tag_name().name() == "par") {
        let Some(ctn) = named(group, "cTn") else {
            continue;
        };
        let conds: Vec<_> = named(ctn, "stCondLst")
            .map(|l| l.children().filter(|c| c.is_element()).collect())
            .unwrap_or_default();
        let click = conds
            .iter()
            .any(|c| c.attribute("delay") == Some("indefinite"))
            && !conds.iter().any(|c| c.attribute("evt") == Some("onBegin"));
        let mut effects = Vec::new();
        let mut all_appear = true;
        let mut fade: f32 = 0.0;
        for e in ctn.descendants().filter(|n| {
            n.tag_name().name() == "cTn"
                && matches!(n.attribute("presetClass"), Some("entr" | "exit"))
        }) {
            let Some(tgt) = e.descendants().find(|n| n.tag_name().name() == "spTgt") else {
                continue;
            };
            let Some(spid) = tgt.attribute("spid") else {
                continue;
            };
            let paras = tgt
                .descendants()
                .find(|n| n.tag_name().name() == "pRg")
                .and_then(|r| {
                    Some((
                        r.attribute("st")?.parse().ok()?,
                        r.attribute("end")?.parse().ok()?,
                    ))
                });
            effects.push(Effect {
                target: Target {
                    spid: spid.to_string(),
                    paras,
                },
                entr: e.attribute("presetClass") == Some("entr"),
            });
            if e.attribute("presetID") != Some("1") {
                all_appear = false;
                let dur = e
                    .descendants()
                    .skip(1)
                    .filter(|n| n.tag_name().name() == "cTn")
                    .find_map(|n| n.attribute("dur")?.parse::<f32>().ok())
                    .unwrap_or(500.0);
                fade = fade.max((dur / 1000.0).clamp(0.2, 1.0));
            }
        }
        if effects.is_empty() {
            // Only video calls or emphasis: nothing to show as a step.
            continue;
        }
        if click {
            b.clicks.push(Click {
                effects,
                fade: if all_appear { 0.0 } else { fade },
            });
        } else if b.clicks.is_empty() {
            b.auto.extend(effects);
        } else if let Some(last) = b.clicks.last_mut() {
            // After the click before (by itself): part of that step.
            last.effects.extend(effects);
        }
    }
    b
}

impl Builds {
    pub fn steps(&self) -> usize {
        self.clicks.len()
    }

    /// The targets not to see after `step` clicks.
    pub fn hidden_at(&self, step: usize) -> Vec<Target> {
        let all = self
            .auto
            .iter()
            .chain(self.clicks.iter().flat_map(|c| c.effects.iter()));
        // Before anything ran: hidden when its first effect brings it in.
        let mut seen: HashMap<&Target, bool> = HashMap::new();
        let mut order: Vec<&Target> = Vec::new();
        for e in all {
            if !seen.contains_key(&e.target) {
                seen.insert(&e.target, !e.entr);
                order.push(&e.target);
            }
        }
        let ran = self
            .auto
            .iter()
            .chain(self.clicks.iter().take(step).flat_map(|c| c.effects.iter()));
        for e in ran {
            seen.insert(&e.target, e.entr);
        }
        order
            .into_iter()
            .filter(|t| seen.get(t) == Some(&false))
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------- a slide per step

/// Invisible text (the space it takes stays).
const CLEAR: &str =
    r#"<P:solidFill><P:srgbClr val="FFFFFF"><P:alpha val="0"/></P:srgbClr></P:solidFill>"#;
const FILLS: &[&str] = &[
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
];
const BULLETS: &[&str] = &["buNone", "buAutoNum", "buChar", "buBlip"];

fn prefix_of(xml: &str, n: roxmltree::Node) -> String {
    let s = &xml[n.range()];
    let name: String = s[1..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
        .collect();
    name.split_once(':')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default()
}

fn qname(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else {
        format!("{prefix}:{local}")
    }
}

fn start_tag_end(xml: &str, n: roxmltree::Node) -> usize {
    let r = n.range();
    r.start + xml[r.clone()].find('>').map_or(0, |i| i + 1)
}

/// The element written again: children named in `remove` left out, `insert`
/// put after the last child named in `after`, else before the first named in
/// `before`, else at the start or the end.
fn rewrite(
    xml: &str,
    n: roxmltree::Node,
    remove: &[&str],
    insert: &str,
    after: &[&str],
    before: &[&str],
    at_end: bool,
) -> (Range<usize>, String) {
    let r = n.range();
    let p = prefix_of(xml, n);
    let end_tag = format!("</{}>", qname(&p, n.tag_name().name()));
    let kids: Vec<_> = n.children().filter(|c| c.is_element()).collect();
    let mut start = xml[r.start..start_tag_end(xml, n)].to_string();
    if kids.is_empty() && start.ends_with("/>") {
        start.truncate(start.len() - 2);
        start.push('>');
    }
    let keep: Vec<_> = kids
        .iter()
        .filter(|k| !remove.contains(&k.tag_name().name()))
        .collect();
    let pos = if let Some(i) = keep
        .iter()
        .rposition(|k| after.contains(&k.tag_name().name()))
    {
        i + 1
    } else if let Some(i) = keep
        .iter()
        .position(|k| before.contains(&k.tag_name().name()))
    {
        i
    } else if at_end {
        keep.len()
    } else {
        0
    };
    let mut out = start;
    for (i, k) in keep.iter().enumerate() {
        if i == pos {
            out.push_str(insert);
        }
        out.push_str(&xml[k.range()]);
    }
    if pos == keep.len() {
        out.push_str(insert);
    }
    out.push_str(&end_tag);
    (r, out)
}

/// Edits that make a paragraph invisible: its text clear, no bullet.
fn hide_paragraph(xml: &str, para: roxmltree::Node, edits: &mut Vec<(Range<usize>, String)>) {
    let a = prefix_of(xml, para);
    let clear = CLEAR.replace(
        "P:",
        &if a.is_empty() {
            String::new()
        } else {
            format!("{a}:")
        },
    );
    let bu_none = format!("<{}/>", qname(&a, "buNone"));
    match named(para, "pPr") {
        Some(ppr) => edits.push(rewrite(
            xml,
            ppr,
            BULLETS,
            &bu_none,
            &[],
            &["tabLst", "defRPr", "extLst"],
            true,
        )),
        None => {
            let at = start_tag_end(xml, para);
            let ppr = qname(&a, "pPr");
            edits.push((at..at, format!("<{ppr}>{bu_none}</{ppr}>")));
        }
    }
    for run in para.children().filter(|c| c.is_element()) {
        match run.tag_name().name() {
            "r" | "fld" | "br" => match named(run, "rPr") {
                Some(rpr) => edits.push(rewrite(xml, rpr, FILLS, &clear, &["ln"], &[], false)),
                None => {
                    let at = start_tag_end(xml, run);
                    let rpr = qname(&a, "rPr");
                    edits.push((at..at, format!("<{rpr}>{clear}</{rpr}>")));
                }
            },
            "endParaRPr" => edits.push(rewrite(xml, run, FILLS, &clear, &["ln"], &[], false)),
            _ => {}
        }
    }
}

/// The slide with `hidden` left out or invisible, without its animations.
pub fn variant(xml: &str, hidden: &[Target]) -> Result<String, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    if let Some(t) = doc.descendants().find(|n| n.tag_name().name() == "timing") {
        edits.push((t.range(), String::new()));
    }
    let shape = |spid: &str| {
        let c = doc
            .descendants()
            .find(|n| n.tag_name().name() == "cNvPr" && n.attribute("id") == Some(spid))?;
        let mut s = c.parent()?.parent()?;
        // A shape inside mc:AlternateContent goes with its wrapper.
        if let Some(p) = s.parent() {
            if matches!(p.tag_name().name(), "Choice" | "Fallback") {
                s = p.parent()?;
            }
        }
        Some(s)
    };
    let gone: Vec<&str> = hidden
        .iter()
        .filter(|t| t.paras.is_none())
        .map(|t| t.spid.as_str())
        .collect();
    for spid in &gone {
        if let Some(s) = shape(spid) {
            edits.push((s.range(), String::new()));
        }
    }
    for t in hidden {
        let Some((st, end)) = t.paras else { continue };
        if gone.contains(&t.spid.as_str()) {
            continue;
        }
        let Some(body) = shape(&t.spid).and_then(|s| named(s, "txBody")) else {
            continue;
        };
        for (i, para) in body
            .children()
            .filter(|c| c.tag_name().name() == "p")
            .enumerate()
        {
            let i = i as u32;
            if i >= st && i <= end {
                hide_paragraph(xml, para, &mut edits);
            }
        }
    }
    // Back to front; nested edits (a paragraph in a removed shape) are not made.
    edits.sort_by_key(|(r, _)| std::cmp::Reverse((r.start, r.end)));
    let mut out = xml.to_string();
    let mut floor = usize::MAX;
    for (r, s) in edits {
        if r.end > floor {
            continue;
        }
        out.replace_range(r.clone(), &s);
        floor = r.start;
    }
    Ok(out)
}

// ---------------------------------------------------------------- the copy

const SLIDE_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const SLIDE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

fn text(files: &[(String, Vec<u8>)], name: &str) -> Option<String> {
    files
        .iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, b)| String::from_utf8(b.clone()).ok())
}

fn put(files: &mut Vec<(String, Vec<u8>)>, name: &str, data: Vec<u8>) {
    match files.iter_mut().find(|(n, _)| n == name) {
        Some(f) => f.1 = data,
        None => files.push((name.to_string(), data)),
    }
}

/// Write a copy of the presentation with a slide for each build step.
/// Returns the steps of each slide in the order of the presentation (hidden
/// slides too, with 0).
pub fn expand(pptx: &Path, to: &Path) -> Result<Vec<usize>, String> {
    let file = std::fs::File::open(pptx).map_err(|e| format!("{}: {e}", pptx.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let mut b = Vec::new();
        f.read_to_end(&mut b).map_err(|e| e.to_string())?;
        files.push((f.name().to_string(), b));
    }
    let pres_name = "ppt/presentation.xml";
    let rels_name = "ppt/_rels/presentation.xml.rels";
    let mut pres = text(&files, pres_name).ok_or("presentation.xml is missing")?;
    let mut pres_rels = text(&files, rels_name).ok_or("presentation.xml.rels is missing")?;
    let mut types = text(&files, "[Content_Types].xml").ok_or("[Content_Types].xml is missing")?;

    // Slide parts in order: (sldId source text, r:id, part).
    let order: Vec<(Range<usize>, String, String)> = {
        let doc = roxmltree::Document::parse(&pres).map_err(|e| e.to_string())?;
        let rdoc = roxmltree::Document::parse(&pres_rels).map_err(|e| e.to_string())?;
        let target: HashMap<&str, &str> = rdoc
            .descendants()
            .filter(|n| n.tag_name().name() == "Relationship")
            .filter_map(|n| Some((n.attribute("Id")?, n.attribute("Target")?)))
            .collect();
        doc.descendants()
            .filter(|n| n.tag_name().name() == "sldId")
            .filter_map(|n| {
                let rid = n
                    .attributes()
                    .find(|a| a.name() == "id" && a.namespace().is_some())?
                    .value()
                    .to_string();
                let t = target.get(rid.as_str())?;
                let part = t
                    .strip_prefix('/')
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("ppt/{t}"));
                Some((n.range(), rid, part))
            })
            .collect()
    };
    let mut next_id: u64 = {
        let doc = roxmltree::Document::parse(&pres).map_err(|e| e.to_string())?;
        doc.descendants()
            .filter(|n| n.tag_name().name() == "sldId")
            .filter_map(|n| {
                n.attributes()
                    .find(|a| a.name() == "id" && a.namespace().is_none())?
                    .value()
                    .parse()
                    .ok()
            })
            .max()
            .unwrap_or(255)
            + 1
    };
    let mut steps = Vec::new();
    let mut new_ids: Vec<(Range<usize>, String)> = Vec::new();
    let mut new_rels = String::new();
    let mut new_types = String::new();
    let mut n = 0;
    for (range, rid, part) in &order {
        let Some(xml) = text(&files, part) else {
            steps.push(0);
            continue;
        };
        let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("{part}: {e}"))?;
        let hidden = doc.root_element().attribute("show") == Some("0");
        let b = parse(&doc);
        if hidden || b.steps() == 0 {
            steps.push(0);
            continue;
        }
        steps.push(b.steps());
        put(
            &mut files,
            part,
            variant(&xml, &b.hidden_at(0))?.into_bytes(),
        );
        let stem = part.trim_end_matches(".xml");
        let rels = text(&files, &crate::pptx::rels_of(part)).unwrap_or_default();
        // The copies share the notes page with the slide: leave that link out.
        let rels = match roxmltree::Document::parse(&rels) {
            Ok(d) => {
                let mut r = rels.clone();
                let mut cut: Vec<Range<usize>> = d
                    .descendants()
                    .filter(|n| {
                        n.tag_name().name() == "Relationship"
                            && n.attribute("Type")
                                .is_some_and(|t| t.ends_with("/notesSlide"))
                    })
                    .map(|n| n.range())
                    .collect();
                cut.sort_by_key(|c| std::cmp::Reverse(c.start));
                for c in cut {
                    r.replace_range(c, "");
                }
                r
            }
            Err(_) => rels,
        };
        let id_text = &pres[range.clone()];
        let mut insert = String::new();
        for k in 1..=b.steps() {
            n += 1;
            let new_part = format!("{stem}-build{k}.xml");
            put(
                &mut files,
                &new_part,
                variant(&xml, &b.hidden_at(k))?.into_bytes(),
            );
            put(
                &mut files,
                &crate::pptx::rels_of(&new_part),
                rels.clone().into_bytes(),
            );
            let new_rid = format!("rIdLanternBuild{n}");
            let rel_target = new_part.strip_prefix("ppt/").unwrap_or(&new_part);
            new_rels.push_str(&format!(
                r#"<Relationship Id="{new_rid}" Type="{SLIDE_REL}" Target="{rel_target}"/>"#
            ));
            new_types.push_str(&format!(
                r#"<Override PartName="/{new_part}" ContentType="{SLIDE_TYPE}"/>"#
            ));
            let old_id = roxmltree::Document::parse(&format!(
                "<x xmlns:r=\"r\" xmlns:p=\"p\">{id_text}</x>"
            ))
            .ok()
            .and_then(|d| {
                d.descendants()
                    .find(|n| n.tag_name().name() == "sldId")
                    .and_then(|n| {
                        n.attributes()
                            .find(|a| a.name() == "id" && a.namespace().is_none())
                            .map(|a| a.value().to_string())
                    })
            })
            .unwrap_or_default();
            let entry = id_text
                .replacen(&format!("\"{rid}\""), &format!("\"{new_rid}\""), 1)
                .replacen(
                    &format!(" id=\"{old_id}\""),
                    &format!(" id=\"{next_id}\""),
                    1,
                );
            next_id += 1;
            insert.push_str(&entry);
        }
        new_ids.push((range.end..range.end, insert));
    }
    if n == 0 {
        return Ok(steps);
    }
    new_ids.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    for (r, s) in new_ids {
        pres.replace_range(r, &s);
    }
    let close = pres_rels
        .rfind("</Relationships>")
        .ok_or("presentation.xml.rels is broken")?;
    pres_rels.insert_str(close, &new_rels);
    let close = types
        .rfind("</Types>")
        .ok_or("[Content_Types].xml is broken")?;
    types.insert_str(close, &new_types);
    put(&mut files, pres_name, pres.into_bytes());
    put(&mut files, rels_name, pres_rels.into_bytes());
    put(&mut files, "[Content_Types].xml", types.into_bytes());

    let tmp = to.with_extension("part");
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let out = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut w = zip::ZipWriter::new(out);
    // The same content gives the same file (the PDF cache goes by it).
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, data) in &files {
        w.start_file(name.as_str(), opts)
            .map_err(|e| e.to_string())?;
        w.write_all(data).map_err(|e| e.to_string())?;
    }
    w.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, to).map_err(|e| e.to_string())?;
    Ok(steps)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A slide: title (2), a picture (3) coming with the first click (fade),
    /// bullets (4) one by one with Appear, the picture going at the end.
    pub(crate) const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Title</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Box"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="4" name="List"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:pPr><a:buChar char="•"/></a:pPr><a:r><a:rPr sz="2400"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:rPr><a:t>One</a:t></a:r></a:p><a:p><a:r><a:rPr lang="de"/><a:t>Two</a:t></a:r><a:endParaRPr/></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:timing><p:tnLst><p:par><p:cTn id="1" nodeType="tmRoot"><p:childTnLst><p:seq><p:cTn id="2" nodeType="mainSeq"><p:childTnLst>
<p:par><p:cTn id="3"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="4"><p:childTnLst><p:par><p:cTn id="5" presetID="10" presetClass="entr" nodeType="clickEffect"><p:childTnLst><p:animEffect><p:cBhvr><p:cTn id="6" dur="700"/><p:tgtEl><p:spTgt spid="3"/></p:tgtEl></p:cBhvr></p:animEffect></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>
<p:par><p:cTn id="7"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="8"><p:childTnLst><p:par><p:cTn id="9" presetID="1" presetClass="entr" nodeType="clickEffect"><p:childTnLst><p:set><p:cBhvr><p:cTn id="10" dur="1"/><p:tgtEl><p:spTgt spid="4"><p:txEl><p:pRg st="0" end="0"/></p:txEl></p:spTgt></p:tgtEl></p:cBhvr></p:set></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>
<p:par><p:cTn id="11"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="12"><p:childTnLst><p:par><p:cTn id="13" presetID="1" presetClass="entr" nodeType="clickEffect"><p:childTnLst><p:set><p:cBhvr><p:cTn id="14" dur="1"/><p:tgtEl><p:spTgt spid="4"><p:txEl><p:pRg st="1" end="1"/></p:txEl></p:spTgt></p:tgtEl></p:cBhvr></p:set></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>
<p:par><p:cTn id="15"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="16"><p:childTnLst><p:par><p:cTn id="17" presetID="10" presetClass="exit" nodeType="clickEffect"><p:childTnLst><p:animEffect><p:cBhvr><p:cTn id="18" dur="300"/><p:tgtEl><p:spTgt spid="3"/></p:tgtEl></p:cBhvr></p:animEffect></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>
</p:childTnLst></p:cTn></p:seq></p:childTnLst></p:cTn></p:par></p:tnLst></p:timing></p:sld>"#;

    fn t(spid: &str, paras: Option<(u32, u32)>) -> Target {
        Target {
            spid: spid.into(),
            paras,
        }
    }

    #[test]
    fn reads_the_clicks() {
        let doc = roxmltree::Document::parse(SLIDE).unwrap();
        let b = parse(&doc);
        assert_eq!(b.steps(), 4);
        assert!((b.clicks[0].fade - 0.7).abs() < 1e-6);
        assert_eq!(b.clicks[1].fade, 0.0, "Appear: at once");
        assert_eq!(b.clicks[1].effects[0].target, t("4", Some((0, 0))));
        assert!(!b.clicks[3].effects[0].entr);
        assert_eq!(
            b.hidden_at(0),
            [t("3", None), t("4", Some((0, 0))), t("4", Some((1, 1)))]
        );
        assert_eq!(b.hidden_at(1), [t("4", Some((0, 0))), t("4", Some((1, 1)))]);
        assert_eq!(b.hidden_at(3), []);
        assert_eq!(b.hidden_at(4), [t("3", None)], "gone again");
    }

    #[test]
    fn video_calls_and_starts_by_themselves_are_no_steps() {
        let v = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/video.pptx"),
        )
        .unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(v)).unwrap();
        let mut s = String::new();
        z.by_name("ppt/slides/slide2.xml")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        let b = parse(&roxmltree::Document::parse(&s).unwrap());
        assert_eq!(b.steps(), 0);
        let auto = SLIDE.replacen(
            r#"<p:cTn id="3"><p:stCondLst><p:cond delay="indefinite"/>"#,
            r#"<p:cTn id="3"><p:stCondLst><p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"/>"#,
            1,
        );
        let b = parse(&roxmltree::Document::parse(&auto).unwrap());
        assert_eq!(b.steps(), 3, "the box comes by itself");
        assert_eq!(b.hidden_at(0).len(), 2);
    }

    #[test]
    fn expands_a_presentation() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/builds.pptx");
        let to = std::env::temp_dir().join(format!("lantern-builds-{}.pptx", std::process::id()));
        assert_eq!(expand(&src, &to).unwrap(), [0, 4, 0]);
        let first = std::fs::read(&to).unwrap();
        // The same again gives the same bytes (the PDF cache goes by them).
        expand(&src, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), first);
        let info = crate::pptx::read(&to).unwrap();
        assert_eq!(info.slides.len(), 7, "slide 2 five times");
        assert!(info.slides[1].title.contains("Agenda"));
        assert!(info.slides[5].title.contains("Agenda"));
        assert!(
            info.slides[6].notes.contains("Folie 3"),
            "{:?}",
            info.slides[6]
        );
        assert!(info.slides[3].notes.is_empty(), "notes stay with the slide");
        let _ = std::fs::remove_file(to);
    }

    #[test]
    fn a_slide_per_step() {
        let doc = roxmltree::Document::parse(SLIDE).unwrap();
        let b = parse(&doc);
        let v0 = variant(SLIDE, &b.hidden_at(0)).unwrap();
        let d = roxmltree::Document::parse(&v0).unwrap();
        assert!(!v0.contains("<p:timing"), "no animations in the copy");
        assert!(!v0.contains(r#"name="Box""#), "the box is not there yet");
        assert!(
            v0.contains(">One<") && v0.contains(">Two<"),
            "text keeps its place"
        );
        // Paragraph one: no bullet, clear text instead of red.
        let p1 = d
            .descendants()
            .filter(|n| n.tag_name().name() == "p")
            .nth(1)
            .unwrap();
        let s = &v0[p1.range()];
        assert!(s.contains("<a:buNone/>") && !s.contains("buChar"), "{s}");
        assert!(
            s.contains(r#"<a:alpha val="0"/>"#) && !s.contains("FF0000"),
            "{s}"
        );
        assert!(s.contains(r#"sz="2400""#), "{s}");
        let p2 = d
            .descendants()
            .filter(|n| n.tag_name().name() == "p")
            .nth(2)
            .unwrap();
        let s = &v0[p2.range()];
        assert!(s.starts_with("<a:p><a:pPr><a:buNone/></a:pPr>"), "{s}");
        assert!(s.contains(r#"<a:rPr lang="de"><a:solidFill>"#), "{s}");
        assert!(s.contains("<a:endParaRPr><a:solidFill>"), "{s}");
        let v3 = variant(SLIDE, &b.hidden_at(3)).unwrap();
        assert!(v3.contains(r#"name="Box""#) && !v3.contains("alpha"));
        assert!(v3.contains(r#"<a:buChar char="•"/>"#));
    }
}
