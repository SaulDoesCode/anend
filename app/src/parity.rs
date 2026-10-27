//! Guards for docs/parity.md: the settings keys and the views the app has. A key or a view
//! that goes away fails these tests until the expected list here and docs/parity.md are
//! changed together, saying why.

/// every key the app's settings file holds (`prefs::Prefs`); add new ones here, and never
/// take one out without a line in docs/parity.md
#[cfg(test)]
pub const SETTINGS: &[&str] = &[
    "palette",
    "colours",
    "font_size",
    "zoom",
    "memory_font",
    "spacing",
    "field_width",
    "mode",
    "gsearch_text",
    "saturate",
    "letters",
    "tint",
    "cls_search",
    "space_bg",
    "space_reversed",
    "space_keep",
    "zen",
    "zen_delay",
    "hide_bar",
    "collapsed",
    "view",
    "save",
    "hub_url",
    "filter_open",
    "filter",
    "keymap",
    "highlight",
    "check_spelling",
    "space_search",
    "hide_list",
    "list_right",
    "white",
    "word_types",
    "space",
    "font",
    "memory_family",
    "top_bar",
    "bottom_bar",
    "bars_swapped",
    "hub_token",
    "read_device",
    "piece_line",
    "piece_lines",
];

/// the views, in their tab order
#[cfg(test)]
pub const VIEWS: &[&str] = &[
    "memory", "space", "lists", "reader", "notes", "tags", "stats", "models", "filters",
];

/// the keymap's action ids (keys.rs): every one the Bevy app had that still applies, and
/// the ones added since
#[cfg(test)]
pub const KEY_ACTIONS: &[&str] = &[
    "goto_memory",
    "goto_space",
    "goto_lists",
    "goto_reader",
    "goto_notes",
    "goto_tags",
    "goto_stats",
    "goto_models",
    "goto_filters",
    "open_settings",
    "open_keys",
    "focus_bar",
    "undo",
    "zoom_in",
    "zoom_out",
    "zoom_reset",
    "save",
    "new_note",
    "toggle_read",
    "toggle_list",
    "note_classifiers",
    "toggle_check",
    "bucket_ordinal",
    "bucket_reversed",
    "bucket_frequency",
    "toggle_gsearch",
    "toggle_saturate",
    "white_mode",
    "letters",
    "classifier_search",
    "word_types",
    "side_ordinal",
    "side_reversed",
    "side_frequency",
    "fly_forward",
    "fly_back",
    "fly_left",
    "fly_right",
    "fly_up",
    "fly_down",
    "roll_left",
    "roll_right",
    "level",
    "back_to_start",
    "slower",
    "faster",
    "walk",
    "superflight",
    "grapple",
    "sneak",
    "new_text",
    "edit_text",
    "delete_text",
    "copy_text",
    "reversed",
    "pin",
    "physics",
    "pause",
    "settle",
    "side_sum",
    "side_reversed_sum",
    "help",
    "hud",
    "details",
    "export",
    "save_map",
    "read_prev",
    "read_next",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_keys_stay() {
        let v = serde_json::to_value(crate::prefs::Prefs::default()).unwrap();
        let have: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in SETTINGS {
            assert!(have.contains(k), "the setting {k} went away: say why in docs/parity.md");
        }
        for k in &have {
            assert!(SETTINGS.contains(k), "a new setting {k}: add it to parity::SETTINGS");
        }
    }

    #[test]
    fn key_actions_stay() {
        let have: Vec<&str> = crate::keys::Action::ALL.iter().map(|a| a.id()).collect();
        for k in KEY_ACTIONS {
            assert!(
                have.contains(k),
                "the key action {k} went away: say why in docs/parity.md"
            );
        }
        for k in &have {
            assert!(
                KEY_ACTIONS.contains(k),
                "a new key action {k}: add it to parity::KEY_ACTIONS"
            );
        }
    }

    /// the memory view's gematria underline (the colour of a text's reversed-sum group)
    /// stays under word-type labels: no rule on a text or its labelled words takes the
    /// underline away or draws one of its own
    #[test]
    fn word_types_keep_the_gematria_underline() {
        let css = include_str!("style.css");
        for rule in css.split('}') {
            let (sel, body) = rule.split_once('{').unwrap_or(("", ""));
            if sel.contains(".art") || sel.contains("ruby.rw") {
                assert!(!body.contains("text-decoration: none"), "{sel} hides the underline");
                assert!(!body.contains("border-bottom"), "{sel} draws a line of its own");
                assert!(!body.contains("text-decoration-color"), "{sel} recolours the underline");
            }
        }
        let mem = include_str!("views/memory.rs");
        assert!(mem.contains("text-decoration: underline; text-decoration-color: {c};"));
        assert!(mem.contains("labels: true, ruby: true"));
    }

    /// the space's texts carry the same gematria grouping underline as the memory view's
    #[test]
    fn space_keeps_the_grouping_underlines() {
        let js = include_str!("space.js");
        assert!(
            js.matches("borderBottomColor = t.under").count() >= 2,
            "space.js must draw t.under"
        );
        let rs = include_str!("views/space.rs");
        assert!(
            rs.contains("\"under\": gem::underline(&h.text"),
            "the space's texts must carry their underline"
        );
        assert!(rs.contains("f[\"under\"] = json!(gem::underline("), "a map's texts too");
    }

    /// spatial saves: blocks keep their tiling UVs, their repeat wrapping and their gizmo,
    /// and the space keeps the window that makes them
    #[test]
    fn blocks_tile_and_have_a_gizmo() {
        let js = include_str!("space.js");
        for needle in [
            "boxGeometry(t)",
            "RepeatWrapping",
            "makeGizmo()",
            "mode: 'scale'",
            "mode: 'move'",
            "kind === 'blockadd'",
            "kind === 'unblock'",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space.rs");
        for needle in [
            "fn Blocks(",
            "\"blocks.list\"",
            "\"blocks.set\"",
            "\"blocks.add\"",
            "show_blocks",
        ] {
            assert!(rs.contains(needle), "space.rs lost {needle}");
        }
    }

    /// taxonomies: the models view's third tab, the tree editor, routing and marking
    #[test]
    fn taxonomies_stay() {
        let cls = include_str!("views/classifiers.rs");
        assert!(cls.contains("(2, \"taxonomies\")") && cls.contains("models: taxonomies"));
        assert!(cls.contains("crate::views::taxonomy::Taxonomies { bar: tax_bar }"));
        let tx = include_str!("views/taxonomy.rs");
        for needle in [
            "\"taxonomy.list\"",
            "\"taxonomy.get\"",
            "\"taxonomy.save\"",
            "\"taxonomy.delete\"",
            "\"taxonomy.route\"",
            "\"taxonomy.mark\"",
            "\"ends here\"",
            "\"none of these\"",
            "fn node_view(",
        ] {
            assert!(tx.contains(needle), "taxonomy.rs lost {needle}");
        }
        assert!(include_str!("../../core/src/s/lib.rs").contains("(\"route\", |rt|"));
    }

    /// s programs: notes tagged s, the right-aligned code mode, word classes, running
    #[test]
    fn s_programs_stay() {
        let hl = include_str!("highlight.rs");
        for needle in ["pub fn s_classes(", "pub fn s_html(", "pub s: bool"] {
            assert!(hl.contains(needle), "highlight.rs lost {needle}");
        }
        let rs = include_str!("views/notes.rs");
        for needle in [
            "\"run the program\"",
            "\"s.run\"",
            "s-run",
            "ops::script::TAG",
            "\"s.debug\"",
            "\"step through the program\"",
            "\"the program's outline\"",
            "goto_req",
            "__anendCompl",
        ] {
            assert!(rs.contains(needle), "notes.rs lost {needle}");
        }
        assert!(include_str!("style.css").contains(".pad.code textarea"));
        assert!(hl.contains("pub fn s_html_with(") && include_str!("views/scode.rs").contains("pub fn outline("));
        // the bar's Run and the command line's run (#11)
        assert!(include_str!("views/scode.rs").contains("pub async fn run_from_bar("));
        assert!(include_str!("views/memory.rs").contains("run_from_bar"));
        assert!(include_str!("views/space.rs").contains("run_from_bar"));
        assert!(include_str!("main.rs").contains("fn run_cli("));
        assert!(include_str!("../../core/src/s/lib.rs").contains("\"anend\" =>"));
        // classifiers and models as words (#18)
        assert!(include_str!("../../core/src/s/lib.rs").contains("\"learn\" => (&[\"anend\"], LEARN)"));
    }

    /// wiring: blocks that cast rays and run programs, the window, playing in the room
    #[test]
    fn wiring_stays() {
        let js = include_str!("space.js");
        for needle in [
            "setWires(list, live)",
            "wireTimers(now)",
            "wireZone(zone, text, event)",
            "showRays(rays)",
            "this.wireFire(id, 'click')",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/wirewin.rs");
        for needle in [
            "fn Wires(",
            "\"wiring.fire\"",
            "\"wiring.set\"",
            "pick targets in the room",
            "▶ play",
            "try it",
        ] {
            assert!(rs.contains(needle), "wirewin.rs lost {needle}");
        }
        let sp = include_str!("views/space.rs");
        assert!(sp.contains("show_wiring") && sp.contains("\"blockfire\""));
    }

    /// prefabs: grouping, the menu with pictures, a copy's controls
    #[test]
    fn prefabs_stay() {
        let js = include_str!("space.js");
        assert!(js.contains("thumbs(list, textures)") && js.contains("part_of"));
        for needle in [
            "startGhost(prefab, parts, textures, name)",
            "aimGhost()",
            "placeGhost()",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/prefabwin.rs");
        for needle in [
            "fn Prefabs(",
            "\"prefabs.make\"",
            "\"prefabs.place\"",
            "\"prefabs.apply\"",
            "\"prefabs.detach\"",
            "pick blocks in the room",
            "…but not the other copies",
            "\"prefabs.starters\"",
        ] {
            assert!(rs.contains(needle), "prefabwin.rs lost {needle}");
        }
        let sp = include_str!("views/space.rs");
        assert!(sp.contains("show_prefabs") && sp.contains("\"prefabspot\"") && sp.contains("\"thumbs\""));
    }

    /// sounds: the window, emitters played by the room
    #[test]
    fn sounds_stay() {
        let js = include_str!("space.js");
        for needle in [
            "playSound(blockId)",
            "startLoops()",
            "hushAll()",
            "soundData(id, data)",
            "hearFrom()",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/soundwin.rs");
        for needle in [
            "fn Sounds(",
            "\"sounds.add\"",
            "\"blocks.set\"",
            "sounds to add",
            "the sound it plays",
        ] {
            assert!(rs.contains(needle), "soundwin.rs lost {needle}");
        }
        let sp = include_str!("views/space.rs");
        assert!(sp.contains("show_sounds") && sp.contains("\"soundwant\""));
    }

    /// space.js defines each method of the room once: a second one replaces the first
    /// without a word (an audio `listen` once shadowed the one that attaches the events)
    #[test]
    fn room_methods_are_unique() {
        let js = include_str!("space.js");
        let mut seen = std::collections::HashSet::new();
        for line in js.lines() {
            let Some(rest) = line.strip_prefix("  ") else { continue };
            if rest.starts_with(' ') || !rest.ends_with(") {") {
                continue;
            }
            let Some((name, _)) = rest.split_once('(') else {
                continue;
            };
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            if ["if", "for", "while", "switch", "catch", "constructor"].contains(&name) {
                continue;
            }
            assert!(seen.insert(name.to_string()), "space.js defines {name} twice");
        }
        assert!(seen.len() > 100);
    }

    /// first-person mode: walking, the grapple, superflight and what it collects
    #[test]
    fn first_person_stays() {
        let js = include_str!("space.js");
        for needle in [
            "setMode(mode)",
            "walkStep(dt)",
            "grapple()",
            "superflight(dt)",
            "collect(a, b)",
            "BODY.mantle",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/flightwin.rs");
        for needle in [
            "fn Collected(",
            "\"mem.saturate\"",
            "\"chains.make\"",
            "\"lists.enlist\"",
            "into the bar",
        ] {
            assert!(rs.contains(needle), "flightwin.rs lost {needle}");
        }
        assert!(include_str!("views/space.rs").contains("\"collected\""));
    }

    /// keyframe animations: the window, the room posing blocks
    #[test]
    fn animations_stay() {
        let js = include_str!("space.js");
        for needle in [
            "animPose(a, p)",
            "applyAnims(now)",
            "previewAnim(id, anim, p)",
            "function cubicBezier(",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/animwin.rs");
        for needle in [
            "fn Anims(",
            "fn Curve(",
            "\"anims.save\"",
            "a key here",
            "the selected block plays it",
        ] {
            assert!(rs.contains(needle), "animwin.rs lost {needle}");
        }
        assert!(include_str!("views/space.rs").contains("show_anims"));
    }

    /// texture recipes: the textures window's steps, faces and the sky
    #[test]
    fn texture_recipes_stay() {
        let js = include_str!("space.js");
        for needle in [
            "recipeCanvas(about",
            "recipeTexture(about",
            "blockMaterial(b)",
            "setSky(about)",
            "preview(id, about",
            "function variantOf(",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/texwin.rs");
        for needle in [
            "fn Textures(",
            "\"textures.save\"",
            "\"blocks.retexture\"",
            "\"space.sky\"",
            "on one face:",
            "as the sky",
        ] {
            assert!(rs.contains(needle), "texwin.rs lost {needle}");
        }
        assert!(include_str!("views/space.rs").contains("show_textures"));
    }

    /// lights baked onto blocks, and sphere blocks
    #[test]
    fn lights_and_spheres_stay() {
        let js = include_str!("space.js");
        for needle in [
            "bake(o)",
            "setLights(list, changed)",
            "showLights(on, picked)",
            "lightSpot()",
            "sphereGeometry(t)",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
        let rs = include_str!("views/space/lightwin.rs");
        for needle in [
            "fn Lights(",
            "\"lights.add\"",
            "\"lights.set\"",
            "\"lights.delete\"",
            "move it here",
        ] {
            assert!(rs.contains(needle), "lightwin.rs lost {needle}");
        }
        let sp = include_str!("views/space.rs");
        for needle in [
            "show_lights",
            "fn ShapeRow(",
            "make it a sphere",
            "fn ZoneRow(",
            "a staging zone",
        ] {
            assert!(sp.contains(needle), "space.rs lost {needle}");
        }
    }

    /// the notes editor's spelling check: asked for as one writes, its flags drawn in the
    /// highlight layer, a click's suggestion put in as an undo step of its own
    #[test]
    fn the_spelling_check_stays() {
        let rs = include_str!("views/notes.rs");
        for needle in [
            "\"grammar.check\"",
            "l.layer.full(&t, colour, &flags_now)",
            "fn SpellPick(",
            "__anendPadSnap(p, true)",
            "check_spelling",
        ] {
            assert!(rs.contains(needle), "notes.rs lost {needle}");
        }
    }

    /// Enter in the space's bar gives the keys back to flying
    #[test]
    fn the_space_bar_lets_go_after_enter() {
        let rs = include_str!("views/space.rs");
        let at = rs.find("\"Enter\" => {").expect("the bar's Enter");
        let arm = &rs[at..at + 700];
        assert!(
            arm.contains("spawn_forever(enter(") && arm.contains("b.blur()"),
            "{arm}"
        );
    }

    /// the room's right-click menu, and copies that are placements, not entries
    #[test]
    fn space_texts_have_a_menu_and_safe_copies() {
        let rs = include_str!("views/space.rs");
        for needle in [
            "fn SpaceMenu(",
            "\"cmd\": \"dup\"",
            "\"cmd\": \"removenode\"",
            "forget the entry",
            "e.shift && !t.is_empty()",
            "a copy that inherits",
            "a detached copy",
            "edit this placement only",
            "follow the entry again",
        ] {
            assert!(rs.contains(needle), "space.rs lost {needle}");
        }
        let js = include_str!("space.js");
        for needle in [
            "placements(entry)",
            "duplicate(node, det)",
            "setOwn(node, text)",
            "removeNode(node)",
            "copyKey(entry)",
        ] {
            assert!(js.contains(needle), "space.js lost {needle}");
        }
    }

    /// the saved scenes: listed, opened, renamed, deleted with an undo
    #[test]
    fn scenes_window_stays() {
        let rs = include_str!("views/space.rs");
        for needle in [
            "fn Scenes(",
            "show_scenes",
            "renamed {n} to {to}",
            "deleted the scene {n}",
            "{n} is back",
        ] {
            assert!(rs.contains(needle), "space.rs lost {needle}");
        }
    }

    /// the command bar's input spans the bar with the same room at both ends, so its
    /// centred text doesn't move when what's beside it changes size
    #[test]
    fn the_bar_text_stays_put() {
        let css = include_str!("style.css");
        assert!(
            css.contains("padding: 0 var(--reserve);") && css.contains("min-width: 15.5cm"),
            "the input keeps equal room at both ends"
        );
        assert!(
            css.contains(".cmdbar .side {\n  position: absolute;"),
            "the side sits over the bar, not beside the input"
        );
    }

    /// the reader's note cards show when each note was written
    #[test]
    fn reader_cards_keep_their_dates() {
        let rs = include_str!("views/reader.rs");
        assert!(rs.contains("short_date(&c)") && rs.contains("written "));
    }

    #[test]
    fn views_stay() {
        let have: Vec<&str> = crate::ui::View::ALL.iter().map(|v| v.name()).collect();
        assert_eq!(have, VIEWS, "the views changed: say why in docs/parity.md");
    }
}
