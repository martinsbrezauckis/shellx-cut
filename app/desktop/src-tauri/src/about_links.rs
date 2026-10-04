//! Fixed About destinations. The engine-served page cannot choose an OS URL.

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AboutDestination {
    Site,
    Github,
    ReleaseNotes,
}

fn valid_release_version(version: &str) -> bool {
    version.len() <= 32
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(super) fn destination_url(
    destination: AboutDestination,
    version: Option<&str>,
) -> Result<String, String> {
    const REPO: &str = "https://github.com/martinsbrezauckis/shellx-cut";
    match destination {
        AboutDestination::Site if version.is_none() => Ok("https://theshellx.com/".into()),
        AboutDestination::Github if version.is_none() => Ok(REPO.into()),
        AboutDestination::ReleaseNotes => match version {
            Some(version) if valid_release_version(version) => {
                Ok(format!("{REPO}/releases/tag/v{version}"))
            }
            Some(_) => Err(
                "The offered release version is invalid; open the latest release notes instead."
                    .into(),
            ),
            None => Ok(format!("{REPO}/releases/latest")),
        },
        _ => Err("A release version is only allowed for release notes.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_are_fixed_and_versions_cannot_change_the_url_boundary() {
        assert_eq!(
            destination_url(AboutDestination::Site, None).unwrap(),
            "https://theshellx.com/"
        );
        assert_eq!(
            destination_url(AboutDestination::Github, None).unwrap(),
            "https://github.com/martinsbrezauckis/shellx-cut"
        );
        assert_eq!(
            destination_url(AboutDestination::ReleaseNotes, None).unwrap(),
            "https://github.com/martinsbrezauckis/shellx-cut/releases/latest"
        );
        assert_eq!(
            destination_url(AboutDestination::ReleaseNotes, Some("0.6.115")).unwrap(),
            "https://github.com/martinsbrezauckis/shellx-cut/releases/tag/v0.6.115"
        );
        for version in [
            "",
            "v0.6.115",
            "0.6",
            "0.6.115/../../other",
            "0.6.115?x=1",
            "0.6.115#x",
            "0.6.115@evil",
            "0.6.115%2Fother",
            "0.6.115-rc.1",
            "1.2.3.4",
            "999999999999999999999999999999999",
        ] {
            assert!(
                destination_url(AboutDestination::ReleaseNotes, Some(version)).is_err(),
                "must refuse {version:?}"
            );
        }
        assert!(destination_url(AboutDestination::Site, Some("0.6.115")).is_err());
        assert!(destination_url(AboutDestination::Github, Some("0.6.115")).is_err());
        assert!(serde_json::from_str::<AboutDestination>("\"other\"").is_err());
    }
}
