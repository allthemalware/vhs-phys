//! Configuration: a preset (or `Condition`) maps to a full `Params`, then TOML
//! text and `key=value` overrides are merged on top. Every physical parameter
//! can therefore be overridden individually, e.g. `tape.defects_per_mm2 = 0.03`
//! or `head1.extra_spacing_um = 0.9`.
//!
//! Accepted tables: `condition` (see `Condition`), `record`, `deck`, `decode`,
//! `tape`, `mech`, `tracking`, `head` (both heads), `head0`, `head1`, and the
//! top-level `seed`.

use crate::condition::Condition;
use crate::pipeline::Params;

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
    let mut base = toml::Value::try_from(p).map_err(|e| e.to_string())?;
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
    base.try_into().map_err(|e| format!("config: {e}"))
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

#[cfg(test)]
mod tests {
    use super::*;

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
