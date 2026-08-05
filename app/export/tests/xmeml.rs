//! FCP7 xmeml v5 (Premiere dialect) structural tests vs
//! examples/minimal-premiere.xml: exploded stereo, link wiring,
//! emit-file-def-once, Resolve's blank <duration></duration>.

mod common;

use common::*;
use cut_export::{export_xml, XmlFormat};
use roxmltree::Document;

#[test]
fn xmeml_matches_known_good_example() {
    let ours_text = export_xml(&scenario(), XmlFormat::Premiere).expect("render xmeml");
    let ours = Document::parse(&ours_text).expect("our output must be well-formed XML");
    let ex_text = example("minimal-premiere.xml");
    let ex = Document::parse(&ex_text).unwrap();

    // Root + sequence header.
    assert_eq!(elems(&ours, "xmeml")[0].attribute("version"), Some("5"));
    let (oseq, eseq) = (elems(&ours, "sequence")[0], elems(&ex, "sequence")[0]);
    assert_attrs_eq(oseq, eseq, &["explodedTracks"], "sequence");
    assert_eq!(child_text(oseq, "name"), child_text(eseq, "name"));
    assert_eq!(child_text(oseq, "duration"), "300", "total timeline frames");

    // Sequence rate: integer timebase + ntsc flag (30 / FALSE here).
    let orate = oseq.children().find(|c| c.has_tag_name("rate")).unwrap();
    let erate = eseq.children().find(|c| c.has_tag_name("rate")).unwrap();
    assert_eq!(child_text(orate, "timebase"), child_text(erate, "timebase"));
    assert_eq!(child_text(orate, "ntsc"), child_text(erate, "ntsc"));

    // All 6 clipitems (video 1-2, exploded audio 3-6): ids and end-exclusive
    // frame ranges — start/end = timeline placement, in/out = source range.
    let (oc, ec) = (elems(&ours, "clipitem"), elems(&ex, "clipitem"));
    assert_eq!(oc.len(), 6, "2 video + 2x2 exploded audio clipitems");
    assert_eq!(oc.len(), ec.len());
    for (o, e) in oc.iter().zip(ec.iter()) {
        assert_eq!(
            o.attribute("id"),
            e.attribute("id"),
            "clipitem id allocation"
        );
        for f in ["name", "enabled", "start", "end", "in", "out"] {
            assert_eq!(
                child_text(*o, f),
                child_text(*e, f),
                "clipitem {:?} field {f}",
                o.attribute("id")
            );
        }
        assert_eq!(
            o.attribute("premiereChannelType"),
            e.attribute("premiereChannelType"),
            "premiereChannelType"
        );
    }

    // Emit-file-def-once: full <file> body only under clipitem-1.
    let files = elems(&ours, "file");
    assert_eq!(files.len(), 6, "one file element per clipitem");
    let full = files[0];
    assert_eq!(full.attribute("id"), Some("file-1"));
    assert_eq!(
        child_text(full, "pathurl"),
        "/home/user/media/talk.mp4",
        "bare absolute path, not URI"
    );
    // Resolve's quirk: blank <duration></duration> must be PRESENT and empty.
    let dur = full
        .children()
        .find(|c| c.is_element() && c.has_tag_name("duration"))
        .expect("blank duration tag");
    assert!(dur.text().unwrap_or("").is_empty());
    // channelcount inside the file def's media/audio.
    assert_eq!(
        child_text(
            elems(&ours, "media")[1]
                .children()
                .find(|c| c.has_tag_name("audio"))
                .unwrap(),
            "channelcount"
        ),
        "2"
    );
    // Every later reference is bare: id attr only, no element children.
    for f in &files[1..] {
        assert_eq!(f.attribute("id"), Some("file-1"));
        assert!(
            !f.children().any(|c| c.is_element()),
            "later file refs must be bare"
        );
    }

    // Link wiring on video clipitem 1: self video link + audio tracks 1,2
    // (clipitem-3 / clipitem-5 per the id arithmetic).
    let links: Vec<_> = oc[0]
        .children()
        .filter(|c| c.has_tag_name("link"))
        .collect();
    let elinks: Vec<_> = ec[0]
        .children()
        .filter(|c| c.has_tag_name("link"))
        .collect();
    assert_eq!(links.len(), 3);
    for (o, e) in links.iter().zip(elinks.iter()) {
        for f in ["linkclipref", "mediatype", "trackindex", "clipindex"] {
            assert_eq!(child_text(*o, f), child_text(*e, f), "link field {f}");
        }
    }
    // Audio clipitems carry NO links in the Premiere dialect.
    assert!(!oc[2].children().any(|c| c.has_tag_name("link")));

    // Exploded stereo: two audio tracks with the exploded attrs +
    // outputchannelindex 1/2; sourcetrack indexes 1 and 2.
    let audio_tracks: Vec<_> = elems(&ours, "track")
        .into_iter()
        .filter(|t| t.attribute("currentExplodedTrackIndex").is_some())
        .collect();
    let ex_audio_tracks: Vec<_> = elems(&ex, "track")
        .into_iter()
        .filter(|t| t.attribute("currentExplodedTrackIndex").is_some())
        .collect();
    assert_eq!(
        audio_tracks.len(),
        2,
        "stereo source must explode into exactly 2 tracks"
    );
    for (o, e) in audio_tracks.iter().zip(ex_audio_tracks.iter()) {
        assert_attrs_eq(
            *o,
            *e,
            &[
                "currentExplodedTrackIndex",
                "totalExplodedTrackCount",
                "premiereTrackType",
            ],
            "audio track",
        );
        assert_eq!(
            child_text(*o, "outputchannelindex"),
            child_text(*e, "outputchannelindex")
        );
    }
    let st: Vec<String> = elems(&ours, "sourcetrack")
        .iter()
        .map(|n| child_text(*n, "trackindex"))
        .collect();
    assert_eq!(st, ["1", "1", "2", "2"], "flattened source channel indexes");

    // Cosmetic-but-proven label on audio clipitems.
    assert_eq!(elems(&ours, "label2").len(), 4);
    assert_eq!(child_text(oc[2], "name"), "talk");
}

#[test]
fn mono_source_is_never_exploded() {
    // Mono trap: a mono source's second exploded track would
    // read a non-existent channel and play silent on one side.
    let mut tl = scenario();
    tl["assets"]["a1"]["probe"]["audio_channels"] = serde_json::json!(1);
    let text = export_xml(&tl, XmlFormat::Premiere).unwrap();
    let doc = Document::parse(&text).unwrap();
    let audio_tracks: Vec<_> = elems(&doc, "track")
        .into_iter()
        .filter(|t| t.attribute("currentExplodedTrackIndex").is_some())
        .collect();
    assert_eq!(
        audio_tracks.len(),
        1,
        "mono must yield exactly ONE audio track"
    );
    assert_eq!(
        audio_tracks[0].attribute("totalExplodedTrackCount"),
        Some("1")
    );
    assert_eq!(audio_tracks[0].attribute("premiereTrackType"), Some("Mono"));
    let item = audio_tracks[0]
        .children()
        .find(|c| c.has_tag_name("clipitem"))
        .unwrap();
    assert_eq!(item.attribute("premiereChannelType"), Some("mono"));
}

#[test]
fn unknown_channel_count_is_not_exploded_as_stereo() {
    let mut tl = scenario();
    tl["assets"]["a1"]["probe"]
        .as_object_mut()
        .unwrap()
        .remove("audio_channels");
    let text = export_xml(&tl, XmlFormat::Premiere).unwrap();
    let doc = Document::parse(&text).unwrap();
    let audio_tracks: Vec<_> = elems(&doc, "track")
        .into_iter()
        .filter(|t| t.attribute("currentExplodedTrackIndex").is_some())
        .collect();
    assert_eq!(
        audio_tracks.len(),
        1,
        "unknown channel count must fail safe as mono, not invented stereo"
    );
    assert_eq!(audio_tracks[0].attribute("premiereTrackType"), Some("Mono"));
}
