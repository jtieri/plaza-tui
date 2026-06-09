use plaza_tui::config::{Config, StreamQuality};
use tempfile::TempDir;

fn write_config_to(dir: &TempDir, config: &Config) {
    let path = dir.path().join("config.toml");
    let content = toml::to_string_pretty(config).unwrap();
    std::fs::write(path, content).unwrap();
}

fn load_config_from(dir: &TempDir) -> Config {
    let path = dir.path().join("config.toml");
    let content = std::fs::read_to_string(path).unwrap();
    toml::from_str(&content).unwrap()
}

#[test]
fn test_config_round_trip() {
    let dir = TempDir::new().unwrap();

    let original = Config {
        stream_quality: StreamQuality::Ogg,
        volume: 0.65,
        image_protocol: Some("kitty".to_string()),
    };

    write_config_to(&dir, &original);
    let loaded = load_config_from(&dir);

    assert_eq!(loaded.stream_quality, original.stream_quality);
    assert!((loaded.volume - original.volume).abs() < 0.001);
    assert_eq!(loaded.image_protocol, original.image_protocol);
}

#[test]
fn test_config_default_values() {
    let config = Config::default();
    // MP3 is the default until Opus/HLS decoding lands (Phase 1): it's the only
    // format this build can decode and the most broadly compatible.
    assert_eq!(config.stream_quality, StreamQuality::Mp3);
    assert!((config.volume - 0.8).abs() < 0.001);
    assert!(config.image_protocol.is_none());
}

#[test]
fn test_stream_quality_urls() {
    assert!(StreamQuality::Hls.stream_url().contains("hls"));
    assert!(StreamQuality::Ogg.stream_url().contains("ogg"));
    assert!(StreamQuality::Mp3.stream_url().contains("mp3"));
}

#[test]
fn test_low_quality_urls_use_underscore_not_slash() {
    // Regression: the live server 404s on `/mp3/low` and `/ogg/low`; the correct
    // paths use an underscore. This was a cause of broken playback.
    assert_eq!(
        StreamQuality::Mp3Low.stream_url(),
        "https://radio.plaza.one/mp3_low"
    );
    assert_eq!(
        StreamQuality::OggLow.stream_url(),
        "https://radio.plaza.one/ogg_low"
    );
    assert!(!StreamQuality::Mp3Low.stream_url().contains("/low"));
    assert!(!StreamQuality::OggLow.stream_url().contains("/low"));
}

#[test]
fn test_all_qualities_supported() {
    // Phase 1: MP3 (symphonia), Opus (libopus), and HLS/AAC (TS demux + symphonia)
    // are all decodable.
    for q in [
        StreamQuality::Mp3,
        StreamQuality::Mp3Low,
        StreamQuality::Ogg,
        StreamQuality::OggLow,
        StreamQuality::Hls,
    ] {
        assert!(q.is_supported(), "{q:?} should be supported");
    }
}

#[test]
fn test_config_serialize_deserialize_all_qualities() {
    for quality in [
        StreamQuality::Hls,
        StreamQuality::Ogg,
        StreamQuality::OggLow,
        StreamQuality::Mp3,
        StreamQuality::Mp3Low,
    ] {
        let config = Config {
            stream_quality: quality.clone(),
            volume: 0.5,
            image_protocol: None,
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.stream_quality, quality);
    }
}
