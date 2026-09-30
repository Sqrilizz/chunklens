use std::{borrow::Cow, collections::BTreeMap};

use anyhow::{Result, ensure};
use fastnbt::borrow::LongArray;
use serde::Deserialize;

use crate::model::ChunkMetrics;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Chunk<'a> {
    #[serde(rename = "DataVersion")]
    data_version: Option<i32>,
    #[serde(rename = "xPos")]
    x: Option<i32>,
    #[serde(rename = "zPos")]
    z: Option<i32>,
    #[serde(rename = "Level", borrow)]
    level: Option<Box<Chunk<'a>>>,
    #[serde(alias = "Entities", borrow)]
    entities: Vec<Entity<'a>>,
    #[serde(alias = "BlockEntities", alias = "TileEntities", borrow)]
    block_entities: Vec<BlockEntity<'a>>,
    #[serde(alias = "Sections", borrow)]
    sections: Vec<Section<'a>>,
    #[serde(
        alias = "block_ticks",
        alias = "BlockTicks",
        alias = "TileTicks",
        alias = "ToBeTicked"
    )]
    block_ticks: Vec<serde::de::IgnoredAny>,
    #[serde(alias = "fluid_ticks", alias = "FluidTicks", alias = "LiquidTicks")]
    fluid_ticks: Vec<serde::de::IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Entity<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(rename = "Passengers", borrow)]
    passengers: Vec<Entity<'a>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct BlockEntity<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(alias = "Items", alias = "items")]
    items: Vec<serde::de::IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Section<'a> {
    #[serde(borrow)]
    block_states: Option<BlockStates<'a>>,
    #[serde(alias = "Palette", borrow)]
    palette: Vec<BlockState<'a>>,
    #[serde(alias = "BlockStates", borrow)]
    data: Option<LongArray<'a>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct BlockStates<'a> {
    #[serde(borrow)]
    palette: Vec<BlockState<'a>>,
    #[serde(borrow)]
    data: Option<LongArray<'a>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct BlockState<'a> {
    #[serde(rename = "Name", alias = "name", borrow)]
    name: Cow<'a, str>,
}

pub fn extract(
    bytes: &[u8],
    dimension: &str,
    x: i32,
    z: i32,
    entities_only: bool,
) -> Result<ChunkMetrics> {
    let root: Chunk<'_> = fastnbt::from_bytes(bytes)?;
    let chunk = root.level.as_deref().unwrap_or(&root);
    let mut metric = ChunkMetrics {
        dimension: dimension.to_owned(),
        chunk_x: chunk.x.unwrap_or(x),
        chunk_z: chunk.z.unwrap_or(z),
        data_version: root.data_version.or(chunk.data_version),
        payload_size: bytes.len() as u64,
        is_oversized: bytes.len() >= 1_048_576,
        scheduled_ticks: (chunk.block_ticks.len() + chunk.fluid_ticks.len()) as u64,
        ..Default::default()
    };
    for entity in &chunk.entities {
        count_entity(entity, &mut metric);
    }
    if !entities_only {
        for entity in &chunk.block_entities {
            let id = entity.id.as_ref();
            if id.is_empty() {
                continue;
            }
            metric.block_entity_count += 1;
            bump(&mut metric.block_entity_types, id, 1);
            metric.stored_items += entity.items.len() as u64;

            match id {
                _ if id.contains("shulker_box") => metric.shulkers += 1,
                "minecraft:hopper" => metric.hoppers += 1,
                "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                    metric.furnaces += 1
                }
                "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" => {
                    metric.chests += 1
                }
                "minecraft:spawner" | "minecraft:mob_spawner" => metric.spawners += 1,
                _ if id.contains("sign") => metric.signs += 1,
                _ if id.contains("skull") || id.contains("head") => metric.skulls += 1,
                _ => {}
            }
        }
        for section in &chunk.sections {
            let (palette, data) = match &section.block_states {
                Some(states) => (&states.palette, states.data),
                None => (&section.palette, section.data),
            };
            count_palette(palette, data, &mut metric)?;
        }
        metric.hoppers = metric.hoppers.max(
            metric
                .block_types
                .get("minecraft:hopper")
                .copied()
                .unwrap_or(0),
        );
    }
    Ok(metric)
}

#[inline]
fn bump(map: &mut BTreeMap<String, u64>, key: &str, count: u64) {
    if let Some(slot) = map.get_mut(key) {
        *slot += count;
    } else {
        map.insert(key.to_owned(), count);
    }
}

fn count_entity(entity: &Entity<'_>, metric: &mut ChunkMetrics) {
    let id = entity.id.as_ref();
    if !id.is_empty() {
        metric.entity_count += 1;
        bump(&mut metric.entity_types, id, 1);
        match id {
            "minecraft:villager" => metric.villagers += 1,
            "minecraft:armor_stand" => metric.armor_stands += 1,
            "minecraft:item" => metric.dropped_items += 1,
            "minecraft:experience_orb" => metric.exp_orbs += 1,
            "minecraft:item_frame" | "minecraft:glow_item_frame" => metric.item_frames += 1,
            "minecraft:hopper_minecart" => {
                metric.hopper_minecarts += 1;
                metric.minecarts += 1;
            }
            _ if id.contains("minecart") => metric.minecarts += 1,
            _ => {}
        }
    }
    for passenger in &entity.passengers {
        count_entity(passenger, metric);
    }
}

fn is_actionable(name: &str) -> bool {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    matches!(
        name,
        "redstone_wire"
            | "repeater"
            | "repeating_command_block"
            | "chain_command_block"
            | "command_block"
            | "comparator"
            | "observer"
            | "piston"
            | "sticky_piston"
            | "hopper"
            | "dropper"
            | "dispenser"
            | "spawner"
            | "mob_spawner"
            | "sculk_sensor"
            | "calibrated_sculk_sensor"
            | "sculk_shrieker"
            | "conduit"
            | "beacon"
            | "daylight_detector"
            | "target"
            | "tripwire"
            | "tripwire_hook"
            | "trapped_chest"
            | "tnt"
            | "kelp"
            | "bamboo"
            | "cactus"
            | "sugar_cane"
    ) || name.contains("redstone")
        || name.contains("piston")
        || name.contains("command")
        || name.contains("hopper")
        || name.contains("sensor")
}

fn count_palette(
    palette: &[BlockState<'_>],
    data: Option<LongArray<'_>>,
    metric: &mut ChunkMetrics,
) -> Result<()> {
    if !palette.iter().any(|state| is_actionable(&state.name)) {
        return Ok(());
    }
    ensure!(palette.len() <= 4096, "block palette exceeds section size");
    if palette.len() == 1 {
        add_block(&palette[0].name, 4096, metric);
        return Ok(());
    }
    let data = data.ok_or_else(|| anyhow::anyhow!("missing packed block states"))?;
    let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(4) as usize;
    let per_long = 64 / bits;

    let mut stack_longs = [0_u64; 512];
    let mut heap_longs;
    let longs: &[u64] = {
        let mut count = 0;
        let mut iter = data.iter();
        while count < 512 {
            if let Some(val) = iter.next() {
                stack_longs[count] = val as u64;
                count += 1;
            } else {
                break;
            }
        }
        if let Some(first_overflow) = iter.next() {
            heap_longs = Vec::with_capacity(1024);
            heap_longs.extend_from_slice(&stack_longs);
            heap_longs.push(first_overflow as u64);
            for val in iter {
                heap_longs.push(val as u64);
            }
            &heap_longs[..]
        } else {
            &stack_longs[..count]
        }
    };

    let padded = longs.len() == 4096_usize.div_ceil(per_long);
    ensure!(
        padded || longs.len() == (4096 * bits).div_ceil(64),
        "invalid packed block state length"
    );
    let mask = (1_u64 << bits) - 1;

    let mut stack_counts = [0_u16; 256];
    let mut heap_counts;
    let counts: &mut [u16] = if palette.len() <= stack_counts.len() {
        &mut stack_counts[..palette.len()]
    } else {
        heap_counts = vec![0_u16; palette.len()];
        &mut heap_counts[..]
    };

    for index in 0..4096 {
        let value = if padded {
            (longs[index / per_long] >> ((index % per_long) * bits)) & mask
        } else {
            let bit = index * bits;
            let shift = bit % 64;
            let mut value = longs[bit / 64] >> shift;
            if shift + bits > 64 {
                value |= longs[bit / 64 + 1] << (64 - shift);
            }
            value & mask
        } as usize;
        let count = counts
            .get_mut(value)
            .ok_or_else(|| anyhow::anyhow!("block state index exceeds palette"))?;
        *count += 1;
    }
    for (state, &count) in palette.iter().zip(counts.iter()) {
        if count > 0 && is_actionable(&state.name) {
            add_block(&state.name, count as u64, metric);
        }
    }
    Ok(())
}

fn add_block(name: &str, count: u64, metric: &mut ChunkMetrics) {
    match name {
        "minecraft:redstone_wire" => metric.redstone_wire += count,
        "minecraft:repeater" => metric.repeaters += count,
        "minecraft:comparator" => metric.comparators += count,
        "minecraft:observer" => metric.observers += count,
        "minecraft:piston" | "minecraft:sticky_piston" => metric.pistons += count,
        _ => {}
    }
    bump(&mut metric.block_types, name, count);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::{LongArray as OwnedLongArray, Value};

    fn encode(mut value: serde_json::Value, longs: Vec<i64>) -> Vec<u8> {
        value["sections"][0]["block_states"]["data"] = serde_json::Value::Null;
        value["sections"][0]["block_states"]
            .as_object_mut()
            .unwrap()
            .remove("data");
        let bytes = fastnbt::to_bytes(&value).unwrap();
        let mut root: Value = fastnbt::from_bytes(&bytes).unwrap();
        if let Value::Compound(root) = &mut root
            && let Value::List(sections) = root.get_mut("sections").unwrap()
            && let Value::Compound(section) = &mut sections[0]
            && let Value::Compound(states) = section.get_mut("block_states").unwrap()
        {
            states.insert("data".into(), Value::LongArray(OwnedLongArray::new(longs)));
        }
        fastnbt::to_bytes(&root).unwrap()
    }

    #[test]
    fn counts_hoppers_once_and_includes_passengers() {
        let mut longs = vec![0_i64; 256];
        longs[0] = 1;
        let bytes = encode(
            serde_json::json!({
                "block_entities": [{"id":"minecraft:hopper"}],
                "Entities": [{"id":"minecraft:boat", "Passengers":[{"id":"minecraft:villager"}]}],
                "sections": [{"block_states":{"palette":[{"Name":"minecraft:air"},{"Name":"minecraft:hopper"}]}}]
            }),
            longs,
        );
        let metric = extract(&bytes, "world", 0, 0, false).unwrap();
        assert_eq!(metric.hoppers, 1);
        assert_eq!(metric.entity_count, 2);
        assert_eq!(metric.villagers, 1);
    }

    #[test]
    fn decodes_both_palette_packing_layouts() {
        let palette: Vec<_> = (0..17).map(|i| serde_json::json!({"Name": if i == 16 {"minecraft:observer"} else {"minecraft:air"}})).collect();
        for padded in [true, false] {
            let mut longs = vec![0_u64; if padded { 4096_usize.div_ceil(12) } else { 320 }];
            for i in [12, 13, 4095] {
                if padded {
                    longs[i / 12] |= 16 << ((i % 12) * 5);
                } else {
                    let bit = i * 5;
                    longs[bit / 64] |= 16 << (bit % 64);
                    if bit % 64 + 5 > 64 {
                        longs[bit / 64 + 1] |= 16 >> (64 - bit % 64);
                    }
                }
            }
            let bytes = encode(
                serde_json::json!({"sections":[{"block_states":{"palette":palette}}]}),
                longs.into_iter().map(|v| v as i64).collect(),
            );
            assert_eq!(extract(&bytes, "world", 0, 0, false).unwrap().observers, 3);
        }
    }

    #[test]
    fn ignores_native_arrays_and_reads_legacy_level() {
        let bytes = fastnbt::to_bytes(&serde_json::json!({"DataVersion":2200,"Level":{"xPos":-2,"zPos":4,"Entities":[{"id":"minecraft:villager"}]}})).unwrap();
        let mut root: Value = fastnbt::from_bytes(&bytes).unwrap();
        if let Value::Compound(fields) = &mut root {
            fields.insert(
                "UnusedLighting".into(),
                Value::ByteArray(fastnbt::ByteArray::new(vec![0; 2048])),
            );
        }
        let bytes = fastnbt::to_bytes(&root).unwrap();
        let m = extract(&bytes, "world", 0, 0, false).unwrap();
        assert_eq!((m.chunk_x, m.chunk_z, m.villagers), (-2, 4, 1));
        assert_eq!(m.data_version, Some(2200));
    }
}
