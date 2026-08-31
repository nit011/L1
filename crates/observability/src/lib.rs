//! Read-only observability (architecture.md §10). Must never gate consensus.

pub mod alerts;
pub mod logging;
pub mod prometheus;
pub mod slo;
pub mod tracing;

#[cfg(test)]
mod grafana_schema {
    #[test]
    fn dashboards_json_is_importable_grafana() {
        let raw = include_str!("../../../configs/grafana/dashboards.json");
        let v: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(v
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap()
            .contains("L1"));
        assert!(v.get("panels").and_then(|p| p.as_array()).unwrap().len() >= 5);
        assert!(v.get("templating").is_some());
        assert_eq!(v.get("schemaVersion").and_then(|x| x.as_u64()), Some(38));
        let text = raw.to_string();
        for needle in [
            "l1_block_interval_ms",
            "l1_finality_ms",
            "l1_gossip_mesh_n",
            "l1_mempool_occupancy",
            "l1_exporter_up",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        assert!(serde_json::from_str::<serde_json::Value>("[").is_err());
    }
}
