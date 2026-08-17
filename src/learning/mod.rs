mod apple_podcast;
mod article;
mod common;
mod rss_podcast;
mod spotify_podcast;
mod vimeo;
mod youtube;

use dioxus_logger::tracing;

/// Detects which kind of learning a URL represents and dispatches to the
/// appropriate handler.  This is the single entry point the UI calls for
/// adding a learning — all URL routing lives here so individual handler
/// modules stay focused on their own media type.
pub async fn run_add_learning(url: &str, date_override: &str, use_ai_summary: bool) -> Result<String, String> {
    let url = url.trim().to_string();

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(
            "Please paste a full URL starting with https://\n\nSupported sources:\n  • YouTube       youtube.com/watch or youtu.be\n  • Apple Podcast podcasts.apple.com\n  • Spotify       open.spotify.com/episode\n  • Vimeo         vimeo.com\n  • RSS feed      .xml / .rss or known feed hosts\n  • Article       any other https:// page".to_string()
        );
    }

    if url.contains("youtube.com/watch") || url.contains("youtu.be/") {
        tracing::debug!("Youtube Entry");
        youtube::run_youtube_learning(&url, date_override, use_ai_summary).await
    } else if url.contains("podcasts.apple.com") {
        tracing::debug!("Apple Podcast Entry");
        apple_podcast::run_apple_podcast(&url, date_override, use_ai_summary).await
    } else if url.contains("open.spotify.com/episode/") {
        tracing::debug!("Spotify Podcast Entry");
        spotify_podcast::run_spotify_podcast(&url, date_override, use_ai_summary).await
    } else if is_rss_feed_url(&url) {
        tracing::debug!("RSS Podcast Entry");
        rss_podcast::run_rss_podcast(&url, date_override, use_ai_summary).await
    } else if is_vimeo_url(&url) {
        tracing::debug!("Vimeo Entry");
        vimeo::run_vimeo(&url, date_override, use_ai_summary).await
    } else {
        tracing::debug!("Default (Article) Entry");
        article::run_article(&url, date_override, use_ai_summary).await
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
