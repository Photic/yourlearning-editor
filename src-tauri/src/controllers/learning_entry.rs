pub use super::youtube_learning::RelayState;

/// Detects which kind of learning a URL represents and dispatches to the
/// appropriate handler.  This is the single Tauri command registered for
/// adding a learning — all URL routing lives here so individual handler
/// modules stay focused on their own media type.
#[tauri::command]
pub async fn run_add_learning(
    app: tauri::AppHandle,
    url: String,
    date_override: String,
    use_ai_summary: bool,
) -> Result<String, String> {
    let url = url.trim().to_string();

    if url.contains("youtube.com/watch") || url.contains("youtu.be/") {
        println!("Youtube Entry");
        super::youtube_learning::run_youtube_learning(&app, &url, &date_override, use_ai_summary).await
    } else if url.contains("podcasts.apple.com") {
        println!("Apple Podcast Entry");
        super::apple_podcast_learning::run_apple_podcast(&app, &url, &date_override, use_ai_summary).await
    } else if url.contains("open.spotify.com/episode/") {
        println!("Spotify Podcast Entry");
        super::spotify_podcast_learning::run_spotify_podcast(&app, &url, &date_override, use_ai_summary).await
    } else if is_rss_feed_url(&url) {
        println!("RSS Podcast Entry");
        super::rss_podcast_learning::run_rss_podcast(&app, &url, &date_override, use_ai_summary).await
    } else if is_vimeo_url(&url) {
        println!("Vimeo Entry");
        super::vimeo_learning::run_vimeo(&app, &url, &date_override, use_ai_summary).await
    } else {
        println!("Default (Article) Entry");
        super::article_learning::run_article(&app, &url, &date_override, use_ai_summary).await
    }
}

/// Returns true if the URL looks like a direct podcast RSS feed rather than a
/// web page.  Heuristics (in priority order):
/// - well-known feed hosting domains
/// - common feed path segments (/feed, /rss, …)
/// - explicit .xml / .rss file extension
fn is_rss_feed_url(url: &str) -> bool {
    let lower = url.to_lowercase();

    // Well-known RSS hosting domains
    let feed_domains = [
        "feeds.simplecast.com",
        "feeds.buzzsprout.com",
        "feeds.transistor.fm",
        "feeds.soundcloud.com",
        "feeds.libsyn.com",
        "feeds.megaphone.fm",
        "feeds.acast.com",
        "feeds.captivate.fm",
        "feeds.podcastmirror.com",
        "anchor.fm/s/",
        "audioboom.com/channels/",
        "rss.art19.com",
        "omny.fm/shows/",
        "pinecast.com/feed/",
        "podcasts.files.bbci.co.uk",
    ];
    if feed_domains.iter().any(|d| lower.contains(d)) {
        return true;
    }

    // Path-segment heuristics
    let feed_segments = ["/feed/", "/feed.xml", "/rss", "/podcast.xml", "/episodes.xml"];
    if feed_segments.iter().any(|s| lower.contains(s)) {
        return true;
    }

    // Explicit .xml / .rss extension (strip query string first)
    let path = lower.split('?').next().unwrap_or(&lower);
    path.ends_with(".xml") || path.ends_with(".rss")
}

/// Returns true for vimeo.com watch pages and player.vimeo.com embed URLs.
fn is_vimeo_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("vimeo.com/")
        && !lower.contains("vimeo.com/channels")
        && !lower.contains("vimeo.com/groups")
        && !lower.contains("vimeo.com/album")
}
