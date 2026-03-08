use plaza_tui::audio::hls::resolve_segment_url;

#[test]
fn test_resolve_segment_url_relative() {
    let base = "https://radio.plaza.one/hls/stream/index.m3u8";
    let segment = "segment_001.ts";
    let result = resolve_segment_url(base, segment);
    assert_eq!(result, "https://radio.plaza.one/hls/stream/segment_001.ts");
}

#[test]
fn test_resolve_segment_url_absolute() {
    let base = "https://radio.plaza.one/hls/index.m3u8";
    let segment = "https://cdn.example.com/hls/segment.ts";
    let result = resolve_segment_url(base, segment);
    assert_eq!(result, "https://cdn.example.com/hls/segment.ts");
}

#[test]
fn test_resolve_segment_url_base_with_trailing_slash() {
    let base = "https://radio.plaza.one/hls/";
    let segment = "stream/seg001.ts";
    let result = resolve_segment_url(base, segment);
    assert_eq!(result, "https://radio.plaza.one/hls/stream/seg001.ts");
}

#[test]
fn test_parse_media_playlist() {
    let m3u8 = b"#EXTM3U\n\
                 #EXT-X-VERSION:3\n\
                 #EXT-X-TARGETDURATION:5\n\
                 #EXTINF:5.0,\n\
                 segment_001.ts\n\
                 #EXTINF:5.0,\n\
                 segment_002.ts\n\
                 #EXTINF:4.8,\n\
                 segment_003.ts\n";

    let result = m3u8_rs::parse_media_playlist_res(m3u8);
    assert!(result.is_ok(), "Media playlist parse failed: {:?}", result.err());
    let playlist = result.unwrap();
    assert_eq!(playlist.segments.len(), 3);
    assert_eq!(playlist.target_duration, 5);
    assert_eq!(playlist.segments[0].uri, "segment_001.ts");
    assert_eq!(playlist.segments[2].uri, "segment_003.ts");
}

#[test]
fn test_parse_master_playlist() {
    let m3u8 = b"#EXTM3U\n\
                 #EXT-X-VERSION:3\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\n\
                 low/index.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=320000,CODECS=\"mp4a.40.2\"\n\
                 high/index.m3u8\n";

    let result = m3u8_rs::parse_master_playlist_res(m3u8);
    assert!(result.is_ok(), "Master playlist parse failed: {:?}", result.err());
    let playlist = result.unwrap();
    assert_eq!(playlist.variants.len(), 2);

    // Highest bandwidth should be 320000
    let highest = playlist.variants.iter().max_by_key(|v| v.bandwidth).unwrap();
    assert_eq!(highest.bandwidth, 320000);
    assert_eq!(highest.uri, "high/index.m3u8");
}

#[test]
fn test_resolve_segment_url_no_path() {
    let base = "https://example.com";
    let segment = "seg.ts";
    let result = resolve_segment_url(base, segment);
    // Should produce a valid URL
    assert!(result.contains("seg.ts"));
}
