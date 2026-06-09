//! Validates our assumptions about how m3u8-rs parses Plaza-shaped playlists.
//! URL resolution and the TS→AAC decode pipeline are unit-tested inside
//! `src/audio/hls.rs` and `src/audio/ts.rs` (the latter against a real fixture).

#[test]
fn test_parse_media_playlist() {
    let m3u8 = b"#EXTM3U\n\
                 #EXT-X-VERSION:3\n\
                 #EXT-X-TARGETDURATION:5\n\
                 #EXT-X-MEDIA-SEQUENCE:42\n\
                 #EXTINF:5.0,\n\
                 segment_001.ts\n\
                 #EXTINF:5.0,\n\
                 segment_002.ts\n\
                 #EXTINF:4.8,\n\
                 segment_003.ts\n";

    let playlist = m3u8_rs::parse_media_playlist_res(m3u8).expect("media playlist parse");
    assert_eq!(playlist.segments.len(), 3);
    assert_eq!(playlist.target_duration, 5);
    assert_eq!(playlist.media_sequence, 42);
    assert_eq!(playlist.segments[0].uri, "segment_001.ts");
    assert_eq!(playlist.segments[2].uri, "segment_003.ts");
}

#[test]
fn test_parse_master_playlist_picks_highest_bandwidth() {
    // Mirrors Plaza's real master playlist shape (AAC variants, relative URIs).
    let m3u8 = b"#EXTM3U\n\
                 #EXT-X-VERSION:3\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=70400,CODECS=\"mp4a.40.2\"\n\
                 aac_lofi.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=105600,CODECS=\"mp4a.40.2\"\n\
                 aac_midfi.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=140800,CODECS=\"mp4a.40.2\"\n\
                 aac_hifi.m3u8\n";

    let playlist = m3u8_rs::parse_master_playlist_res(m3u8).expect("master playlist parse");
    assert_eq!(playlist.variants.len(), 3);
    let highest = playlist
        .variants
        .iter()
        .max_by_key(|v| v.bandwidth)
        .unwrap();
    assert_eq!(highest.bandwidth, 140800);
    assert_eq!(highest.uri, "aac_hifi.m3u8");
}
