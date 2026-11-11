//! Configuration: a preset (or `Condition`) maps to a full `Params`, then TOML
//! text and `key=value` overrides are merged on top. Every physical parameter
//! can therefore be overridden individually, e.g. `tape.defects_per_mm2 = 0.03`
//! or `head1.extra_spacing_um = 0.9`.
//!
//! Accepted tables: `condition` (see `Condition`), `record`, `deck`, `decode`,
//! `tape`, `mech`, `tracking`, `head` (both heads), `head0`, `head1`, and the
//! top-level `seed` and `standard`. `build` refuses what the engine cannot play: another
//! engine's standard, composite video into a PAL television, and EP in the 625-line engine.

use super::condition::Condition;
use super::consts::{HAS_EP, STANDARD};
use super::format::TapeSpeed;
use super::pipeline::Params;
use crate::standard::TvStandard;

fn merge(base: &mut toml::Value, over: toml::Value) {
    match (base, over) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(bv) => merge(bv, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

/// Apply TOML overrides on top of `p`.
pub fn apply_overrides(p: &Params, text: &str) -> Result<Params, String> {
    let mut over: toml::Table = toml::from_str(text).map_err(|e| format!("config: {e}"))?;
    // TOML integers are signed 64-bit, so a seed above i64::MAX (derived seeds often are)
    // cannot make the round trip: keep it aside unless the text sets a new one.
    let keep_seed = (!over.contains_key("seed")).then_some(p.seed);
    // the outside world's copy of the seed follows `seed`, so it is set aside the same way
    let mut aside = p.clone();
    aside.seed = 0;
    aside.interference.seed = 0;
    let mut base = toml::Value::try_from(aside).map_err(|e| e.to_string())?;
    // head / head0 / head1 shorthands -> heads[0], heads[1]
    let mut heads: [toml::Table; 2] = [toml::Table::new(), toml::Table::new()];
    if let Some(toml::Value::Table(t)) = over.remove("head") {
        for h in heads.iter_mut() {
            *h = t.clone();
        }
    }
    for (k, i) in [("head0", 0usize), ("head1", 1usize)] {
        if let Some(toml::Value::Table(t)) = over.remove(k) {
            for (a, b) in t {
                heads[i].insert(a, b);
            }
        }
    }
    over.remove("condition");
    merge(&mut base, toml::Value::Table(over));
    if let toml::Value::Table(bt) = &mut base {
        if let Some(toml::Value::Array(arr)) = bt.get_mut("heads") {
            for (i, h) in heads.into_iter().enumerate() {
                if !h.is_empty() {
                    merge(&mut arr[i], toml::Value::Table(h));
                }
            }
        }
    }
    let mut out: Params = base.try_into().map_err(|e| format!("config: {e}"))?;
    if let Some(s) = keep_seed {
        out.seed = s;
    }
    out.interference.seed = out.seed;
    check_standard(out)
}

/// Build parameters from an optional preset name and optional TOML text. A
/// `[condition]` table in the text refines the preset before mapping.
pub fn build(preset: Option<&str>, text: &str, seed: u64) -> Result<Params, String> {
    let mut cond = match preset {
        Some(n) => Condition::preset(n).ok_or_else(|| format!("unknown preset '{n}' (try: {})", Condition::preset_names().join(", ")))?,
        None => Condition::default(),
    };
    let table: toml::Table = toml::from_str(text).map_err(|e| format!("config: {e}"))?;
    if let Some(c) = table.get("condition") {
        let mut base = toml::Value::try_from(&cond).map_err(|e| e.to_string())?;
        merge(&mut base, c.clone());
        cond = base.try_into().map_err(|e| format!("condition: {e}"))?;
    }
    let mut p = cond.to_params(seed);
    if let Some(toml::Value::Integer(s)) = table.get("seed") {
        p.seed = *s as u64;
    }
    apply_overrides(&p, text)
}

/// What this engine can play: the 625-line engine records and plays PAL only, into a PAL
/// television; the 525-line engine records NTSC and plays it into an NTSC television, or, with
/// `standard = "pal60"`, into a PAL television. The television follows the standard (whatever
/// `decode.tv` was set to).
fn check_standard(mut p: Params) -> Result<Params, String> {
    match STANDARD {
        TvStandard::Pal => {
            if p.standard != TvStandard::Pal {
                return Err(format!("the 625-line engine plays only standard = \"pal\" (got \"{}\")", p.standard.name()));
            }
        }
        _ => {
            if p.standard == TvStandard::Pal {
                return Err("standard = \"pal\" is the 625-line engine, not a setting of the 525-line one".into());
            }
        }
    }
    p.decode.tv = p.standard.colour_tv();
    if p.speed == TapeSpeed::Ep && !HAS_EP {
        return Err("EP is not a 625-line VHS speed (SP and LP only)".into());
    }
    Ok(p)
}

/// Turn `key=value` strings (as given to `--set`) into TOML text. If a key is
/// repeated the last value wins.
pub fn sets_to_toml(sets: &[String]) -> Result<String, String> {
    let mut items: Vec<(String, String)> = Vec::new();
    for s in sets {
        let (k, v) = s.split_once('=').ok_or_else(|| format!("--set expects key=value, got '{s}'"))?;
        let (k, v) = (k.trim().to_string(), v.trim());
        // `tape.defects_per_mm2=0.03` -> dotted keys are valid TOML; quote bare words
        let is_lit = v.parse::<f64>().is_ok() || v == "true" || v == "false" || v.starts_with('[') || v.starts_with('"');
        let v = if is_lit { v.to_string() } else { format!("\"{v}\"") };
        match items.iter_mut().find(|(ek, _)| *ek == k) {
            Some(e) => e.1 = v,
            None => items.push((k, v)),
        }
    }
    Ok(items.into_iter().map(|(k, v)| format!("{k} = {v}\n")).collect())
}

/// Parameters of every deck in a chain of `generations` dubs. Generation 0 is exactly
/// `build(preset, text, seed)`; each later generation is a different tape and deck (its
/// own seed, so its own defects, wear draws and timebase) with the same condition.
pub fn generation_params(preset: Option<&str>, text: &str, seed: u64, generations: usize) -> Result<Vec<Params>, String> {
    if generations == 0 {
        return Err("generations must be at least 1".into());
    }
    // A top-level `seed` in the text is the seed of the whole chain: generation 0 takes it
    // (as `build` does) and the later generations derive theirs from it. It is removed from
    // the text of those later generations, or it would replace their own seeds.
    let mut table: toml::Table = toml::from_str(text).map_err(|e| format!("config: {e}"))?;
    let base = match table.remove("seed") {
        Some(toml::Value::Integer(s)) => s as u64,
        _ => seed,
    };
    let later_text = toml::to_string(&table).map_err(|e| format!("config: {e}"))?;
    (0..generations)
        .map(|g| {
            if g == 0 {
                return build(preset, text, seed);
            }
            // a different seed per dub, derived from the chain seed so a chain is reproducible
            let seed_g = crate::rng::mix(base ^ (g as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut p = build(preset, &later_text, seed_g)?;
            p.seed = seed_g;
            Ok(p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::standard::ColourTv;

    #[test]
    fn overrides_reach_individual_fields() {
        let p = build(Some("worn"), "[tape]\ndefects_per_mm2 = 0.5\n[head1]\nextra_spacing_um = 0.9\n[head]\ncnr_db = 31.0\n", 3).unwrap();
        assert_eq!(p.tape.defects_per_mm2, 0.5);
        assert_eq!(p.heads[1].extra_spacing_um, 0.9);
        assert_eq!(p.heads[0].cnr_db, 31.0);
        assert_eq!(p.heads[1].cnr_db, 31.0);
        // untouched values come from the preset mapping
        assert!(p.mech.drum_ns > 20.0);
    }

    #[test]
    fn condition_table_refines_the_preset() {
        let a = build(Some("good"), "", 1).unwrap();
        let b = build(Some("good"), "[condition]\ndeck_age_years = 40.0\ndeck_temp_c = 70.0\n", 1).unwrap();
        assert!(b.deck.supply.hum_ire > a.deck.supply.hum_ire);
    }

    #[test]
    fn seeds_above_the_toml_integer_range_survive_overrides() {
        let big = u64::MAX - 7;
        let p = build(Some("worn"), "[tape]\npasses = 3.0\n", big).unwrap();
        assert_eq!(p.seed, big);
        assert_eq!(p.tape.passes, 3.0);
        assert_eq!(apply_overrides(&p, "seed = 5\n").unwrap().seed, 5);
    }

    #[test]
    fn a_seed_in_the_text_is_the_seed_of_the_whole_chain() {
        let from_text = generation_params(Some("worn"), "seed = 7\n[tape]\npasses = 3.0\n", 1, 3).unwrap();
        let from_arg = generation_params(Some("worn"), "[tape]\npasses = 3.0\n", 7, 3).unwrap();
        // generation 0 is exactly what `build` gives for the same text
        assert_eq!(format!("{:?}", from_text[0]), format!("{:?}", build(Some("worn"), "seed = 7\n[tape]\npasses = 3.0\n", 1).unwrap()));
        assert_eq!(from_text[0].seed, 7);
        // later generations derive their own seeds from it, as from `seed = 7` given directly
        for g in 1..3 {
            assert_eq!(format!("{:?}", from_text[g]), format!("{:?}", from_arg[g]));
            assert_eq!(from_text[g].tape.passes, 3.0);
        }
        assert!(from_text[1].seed != 7 && from_text[2].seed != 7 && from_text[1].seed != from_text[2].seed);
    }

    #[test]
    fn the_television_follows_the_standard() {
        // an explicit `decode.tv` never overrides the standard's own television
        let n = build(None, "[decode]\ntv = \"pal\"\n", 1);
        if STANDARD == TvStandard::Pal {
            assert_eq!(n.unwrap().decode.tv, ColourTv::Pal);
            assert!(build(None, "standard = \"ntsc\"\n", 1).is_err());
            assert!(build(None, "standard = \"pal60\"\n", 1).is_err());
            assert!(build(None, "[composite]\nenabled = true\n", 1).is_ok());
        } else {
            let n = n.unwrap();
            assert_eq!((n.standard, n.decode.tv), (TvStandard::Ntsc, ColourTv::Ntsc));
            let p = build(None, "standard = \"pal60\"\n[decode]\ntv = \"ntsc\"\n", 1).unwrap();
            assert_eq!((p.standard, p.decode.tv), (TvStandard::Pal60, ColourTv::Pal));
            assert!(build(None, "standard = \"pal\"\n", 1).is_err());
            assert!(build(None, "standard = \"pal60\"\n[composite]\nenabled = true\n", 1).is_ok());
            assert!(build(None, "[composite]\nenabled = true\n", 1).is_ok());
        }
    }

    #[test]
    fn set_strings_and_errors() {
        let t = sets_to_toml(&["tape.passes=12".into(), "decode.comb=false".into()]).unwrap();
        let p = build(None, &t, 1).unwrap();
        assert_eq!(p.tape.passes, 12.0);
        assert!(!p.decode.comb);
        assert!(build(Some("nope"), "", 1).is_err());
        // repeated keys: the last one wins
        let t = sets_to_toml(&["tape.passes=1".into(), "tape.passes=9".into()]).unwrap();
        assert_eq!(build(None, &t, 1).unwrap().tape.passes, 9.0);
        assert!(build(None, "[tape]\nbogus_field = 1\n", 1).is_ok(), "unknown fields are ignored by serde defaults");
    }
}
