//! Fetch a student's GitHub repository into a working directory.
//!
//! * [`url`] — parse a GitHub URL into `owner`/`repo` and build the tarball endpoint.
//! * [`extract`] — safely unpack the `tar.gz` (strip the `<repo>-<sha>/` prefix, reject
//!   traversal, skip symlinks, enforce byte/file caps).
//! * [`download`] — the thin networked layer tying the two together.

mod download;
mod error;
mod extract;
mod url;

pub use download::{FetchOutcome, fetch_repo};
pub use error::FetchError;
pub use extract::{Caps, ExtractStats, extract_tar_gz, sanitize_entry_path};
pub use url::{RepoRef, parse_github_url, tarball_url};
