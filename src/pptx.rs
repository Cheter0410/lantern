//! What Lantern reads from a .pptx itself (the pictures come from LibreOffice):
//! slide order, hidden slides, titles, speaker notes and transitions.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct SlideInfo {
    /// Number in the original presentation (1-based, hidden slides counted).
    pub number: usize,
    pub title: String,
    pub notes: String,
    pub hidden: bool,
    pub transition: Transition,
}

/// How a slide appears (PowerPoint's many effects become a fade).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    Cut,
    Fade(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// Slide size in EMU (aspect ratio).
    pub size: (u64, u64),
    pub slides: Vec<SlideInfo>,
}

impl Info {
    /// The slides LibreOffice puts into the PDF (hidden ones are left out).
    pub fn visible(&self) -> Vec<SlideInfo> {
        self.slides.iter().filter(|s| !s.hidden).cloned().collect()
    }
}

struct Zip(zip::ZipArchive<std::fs::File>);

impl Zip {
    fn text(&mut self, name: &str) -> Option<String> {
        let mut f = self.0.by_name(name).ok()?;
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    }
}

/// `ppt/slides/slide1.xml` -> `ppt/slides/_rels/slide1.xml.rels`
fn rels_of(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolve a relationship target relative to the part it belongs to.
fn resolve(part: &str, target: &str) -> String {
    if let Some(t) = target.strip_prefix('/') {
        return t.to_string();
    }
    let mut segs: Vec<&str> = part.split('/').collect();
    segs.pop();
    for s in target.split('/') {
        match s {
            ".." => {
                segs.pop();
            }
            "." | "" => {}
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// Relationships of a part: id -> (type, resolved target).
fn rels(zip: &mut Zip, part: &str) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    let Some(xml) = zip.text(&rels_of(part)) else {
        return out;
    };
    let Ok(doc) = roxmltree::Document::parse(&xml) else {
        return out;
    };
    for n in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Relationship")
    {
        if let (Some(id), Some(ty), Some(t)) = (
            n.attribute("Id"),
            n.attribute("Type"),
            n.attribute("Target"),
        ) {
            let target = if n.attribute("TargetMode") == Some("External") {
                t.to_string()
            } else {
                resolve(part, t)
            };
            out.insert(id.to_string(), (ty.to_string(), target));
        }
    }
    out
}

fn attr_ns<'a>(n: &roxmltree::Node<'a, '_>, local: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
}

/// Text of the shapes with one of the placeholder types, paragraphs on lines.
fn placeholder_text(doc: &roxmltree::Document, types: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    for sp in doc.descendants().filter(|n| n.tag_name().name() == "sp") {
        let ph = sp.descendants().find(|n| n.tag_name().name() == "ph");
        let ty = ph.and_then(|p| p.attribute("type")).unwrap_or("body");
        if ph.is_none() || !types.contains(&ty) {
            continue;
        }
        for p in sp.descendants().filter(|n| n.tag_name().name() == "p") {
            let line: String = p
                .descendants()
                .filter(|n| n.tag_name().name() == "t")
                .filter_map(|n| n.text())
                .collect();
            if !line.trim().is_empty() {
                out.push(line.trim().to_string());
            }
        }
    }
    out.join("\n")
}

fn transition(doc: &roxmltree::Document) -> Transition {
    // PowerPoint writes the transition twice (new and old style) inside
    // mc:AlternateContent; the first one found carries the newest details.
    let Some(t) = doc
        .descendants()
        .find(|n| n.tag_name().name() == "transition")
    else {
        return Transition::Cut;
    };
    let effect = t
        .children()
        .find(|c| c.is_element())
        .map(|c| c.tag_name().name().to_string());
    let secs = attr_ns(&t, "dur")
        .and_then(|d| d.parse::<f32>().ok())
        .map(|ms| ms / 1000.0)
        .or_else(|| {
            attr_ns(&t, "spd").map(|s| match s {
                "slow" => 1.0,
                "med" => 0.75,
                _ => 0.5,
            })
        })
        .unwrap_or(0.7);
    match effect.as_deref() {
        None | Some("cut") => Transition::Cut,
        Some(_) => Transition::Fade(secs.clamp(0.05, 10.0)),
    }
}

pub fn read(path: &Path) -> Result<Info, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip =
        Zip(zip::ZipArchive::new(file).map_err(|_| "Not a PowerPoint file (.pptx)".to_string())?);
    let pres = "ppt/presentation.xml";
    let xml = zip
        .text(pres)
        .ok_or("Not a PowerPoint file (ppt/presentation.xml is missing)")?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("presentation.xml: {e}"))?;
    let size = doc
        .descendants()
        .find(|n| n.tag_name().name() == "sldSz")
        .and_then(|n| {
            Some((
                n.attribute("cx")?.parse().ok()?,
                n.attribute("cy")?.parse().ok()?,
            ))
        })
        .unwrap_or((12_192_000, 6_858_000));
    let pres_rels = rels(&mut zip, pres);
    let ids: Vec<String> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "sldId")
        .filter_map(|n| {
            // r:id (the relationship), not the plain numeric id.
            n.attributes()
                .find(|a| a.name() == "id" && a.namespace().is_some())
                .map(|a| a.value().to_string())
        })
        .collect();
    let mut slides = Vec::new();
    for (i, rid) in ids.iter().enumerate() {
        let Some((_, part)) = pres_rels.get(rid) else {
            continue;
        };
        let part = part.clone();
        let Some(sx) = zip.text(&part) else { continue };
        let Ok(sd) = roxmltree::Document::parse(&sx) else {
            continue;
        };
        let hidden = sd.root_element().attribute("show") == Some("0");
        let mut title = placeholder_text(&sd, &["title", "ctrTitle"]);
        if title.is_empty() {
            // No title placeholder: the first text on the slide.
            title = sd
                .descendants()
                .filter(|n| n.tag_name().name() == "p")
                .map(|p| {
                    p.descendants()
                        .filter(|n| n.tag_name().name() == "t")
                        .filter_map(|n| n.text())
                        .collect::<String>()
                })
                .find(|t| !t.trim().is_empty())
                .unwrap_or_default()
                .trim()
                .to_string();
        }
        let srels = rels(&mut zip, &part);
        let notes = srels
            .values()
            .find(|(ty, _)| ty.ends_with("/notesSlide"))
            .and_then(|(_, t)| zip.text(t))
            .and_then(|x| {
                roxmltree::Document::parse(&x)
                    .ok()
                    .map(|d| placeholder_text(&d, &["body"]))
            })
            .unwrap_or_default();
        slides.push(SlideInfo {
            number: i + 1,
            title: title.lines().next().unwrap_or_default().to_string(),
            notes,
            hidden,
            transition: transition(&sd),
        });
    }
    if slides.is_empty() {
        return Err("The presentation has no slides.".into());
    }
    Ok(Info { size, slides })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_targets() {
        assert_eq!(
            resolve("ppt/presentation.xml", "slides/slide1.xml"),
            "ppt/slides/slide1.xml"
        );
        assert_eq!(
            resolve("ppt/slides/slide1.xml", "../notesSlides/notesSlide1.xml"),
            "ppt/notesSlides/notesSlide1.xml"
        );
        assert_eq!(
            rels_of("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
    }

    #[test]
    fn reads_the_demo_presentation() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/demo.pptx");
        let info = read(&p).unwrap();
        assert_eq!(info.slides.len(), 3);
        assert!(
            info.slides[0].notes.contains("Folie 1"),
            "{:?}",
            info.slides[0]
        );
        assert_eq!(info.size, (12_191_695, 6_858_000));
    }

    #[test]
    fn transitions() {
        let t = |x: &str| transition(&roxmltree::Document::parse(x).unwrap());
        let x = r#"<p:sld xmlns:p="p"><p:transition spd="slow"><p:fade/></p:transition></p:sld>"#;
        assert_eq!(t(x), Transition::Fade(1.0));
        let x = r#"<p:sld xmlns:p="p" xmlns:p14="q"><p:transition p14:dur="300"><p:push/></p:transition></p:sld>"#;
        assert_eq!(t(x), Transition::Fade(0.3));
        let x = r#"<p:sld xmlns:p="p"><p:transition/></p:sld>"#;
        assert_eq!(t(x), Transition::Cut);
        let x = r#"<p:sld xmlns:p="p"/>"#;
        assert_eq!(t(x), Transition::Cut);
    }
}
