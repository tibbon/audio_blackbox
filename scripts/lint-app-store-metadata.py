#!/usr/bin/env python3
"""Lint App Store Connect metadata against Apple's OpenAPI spec.

Catches schema drift before a `fastlane metadata` / `fastlane submit_review`
dispatch fails on the runner. Specifically targets the failure modes we
hit shipping 1.1.0:

  - age_rating_config.json keys that have been renamed by Apple
    (UPPER_SNAKE_CASE legacy → camelCase canonical, e.g.
    SEXUAL_CONTENT_GRAPHIC_NUDITY → sexualContentGraphicAndNudity).
  - metadata/<locale>/*.txt files exceeding well-known character
    limits that Apple enforces server-side.

Apple's OpenAPI spec lives at:
  https://developer.apple.com/sample-code/app-store-connect/app-store-connect-openapi-specification.zip

Cached at .cache/asc-openapi.json. Pass --refresh-spec to redownload.

Exit codes:
  0  no errors (warnings allowed)
  1  one or more errors; the next deploy will fail at deliver
"""

import argparse
import json
import sys
import urllib.request
import zipfile
from io import BytesIO
from pathlib import Path

SPEC_URL = (
    "https://developer.apple.com/sample-code/app-store-connect/"
    "app-store-connect-openapi-specification.zip"
)
REPO_ROOT = Path(__file__).resolve().parent.parent
CACHE_PATH = REPO_ROOT / ".cache" / "asc-openapi.json"

# Per-field character limits documented at
# https://developer.apple.com/help/app-store-connect/manage-app-information/.
# Not declared in the OpenAPI spec but enforced server-side, so a hardcoded
# table is the practical option.
TEXT_LIMITS = {
    "name.txt": 30,
    "subtitle.txt": 30,
    "keywords.txt": 100,
    "description.txt": 4000,
    "release_notes.txt": 4000,
    "promotional_text.txt": 170,
    "marketing_url.txt": 255,
    "support_url.txt": 255,
    "privacy_url.txt": 255,
}

# Mirror of fastlane's Spaceship::ConnectAPI::AgeRatingDeclaration
# legacy → canonical translation. The translation is best-effort:
# entries mapped to None have no equivalent in the current ASC API
# (Apple removed the rating concept) and SHOULD be removed from local
# config entirely. Confirmed against fastlane master + the OpenAPI spec.
ITC_TO_CANONICAL = {
    "CARTOON_FANTASY_VIOLENCE": "violenceCartoonOrFantasy",
    "REALISTIC_VIOLENCE": "violenceRealistic",
    "PROLONGED_GRAPHIC_SADISTIC_REALISTIC_VIOLENCE":
        "violenceRealisticProlongedGraphicOrSadistic",
    "PROFANITY_CRUDE_HUMOR": "profanityOrCrudeHumor",
    "MATURE_SUGGESTIVE": "matureOrSuggestiveThemes",
    "HORROR": "horrorOrFearThemes",
    "MEDICAL_TREATMENT_INFO": "medicalOrTreatmentInformation",
    "ALCOHOL_TOBACCO_DRUGS": "alcoholTobaccoOrDrugUseOrReferences",
    "GAMBLING": "gambling",
    "GAMBLING_CONTESTS": "contests",
    "SEXUAL_CONTENT_GRAPHIC_NUDITY": "sexualContentGraphicAndNudity",
    "SEXUAL_CONTENT_OR_NUDITY": "sexualContentOrNudity",
    "GRAPHIC_SEXUAL_CONTENT_NUDITY": None,
    "UNRESTRICTED_WEB_ACCESS": "unrestrictedWebAccess",
}


def load_spec(refresh: bool) -> dict:
    if not refresh and CACHE_PATH.exists():
        return json.loads(CACHE_PATH.read_text())
    print(f"Downloading OpenAPI spec from {SPEC_URL}", file=sys.stderr)
    with urllib.request.urlopen(SPEC_URL, timeout=30) as resp:
        zip_bytes = resp.read()
    with zipfile.ZipFile(BytesIO(zip_bytes)) as zf:
        json_name = next(
            n for n in zf.namelist()
            if n.endswith(".json") and not n.startswith("__MACOSX")
        )
        spec_text = zf.read(json_name).decode("utf-8")
    spec = json.loads(spec_text)
    CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
    CACHE_PATH.write_text(json.dumps(spec))
    return spec


def age_rating_attributes(spec: dict) -> set:
    return set(
        spec["components"]["schemas"]["AgeRatingDeclaration"]
            ["properties"]["attributes"]["properties"].keys()
    )


def lint_age_rating(spec: dict, errors: list, warnings: list) -> None:
    config_path = REPO_ROOT / "BlackBoxApp" / "fastlane" / "age_rating_config.json"
    if not config_path.exists():
        return
    canonical = age_rating_attributes(spec)
    config = json.loads(config_path.read_text())
    rel = config_path.relative_to(REPO_ROOT)

    for key in config:
        if key in canonical:
            continue
        if key in ITC_TO_CANONICAL:
            translated = ITC_TO_CANONICAL[key]
            if translated is None:
                errors.append(
                    f"{rel}: '{key}' has no equivalent in the current ASC "
                    f"API. Apple removed this rating dimension; remove the "
                    f"key from the file."
                )
            elif translated in canonical:
                warnings.append(
                    f"{rel}: '{key}' is the legacy iTunes Connect name. "
                    f"Use '{translated}' (current canonical name)."
                )
            else:
                errors.append(
                    f"{rel}: '{key}' translates to '{translated}' but that "
                    f"attribute is not in the current AgeRatingDeclaration "
                    f"schema. Apple may have renamed it again."
                )
        else:
            errors.append(
                f"{rel}: '{key}' is not a known AgeRatingDeclaration "
                f"attribute and has no legacy translation. ASC will reject "
                f"this with 'unknown attribute'."
            )


def lint_metadata_lengths(errors: list) -> None:
    metadata_dir = REPO_ROOT / "BlackBoxApp" / "fastlane" / "metadata"
    if not metadata_dir.exists():
        return
    for locale_dir in sorted(metadata_dir.iterdir()):
        if not locale_dir.is_dir() or locale_dir.name.startswith("."):
            continue
        for filename, limit in TEXT_LIMITS.items():
            path = locale_dir / filename
            if not path.exists():
                continue
            length = len(path.read_text().strip())
            if length > limit:
                errors.append(
                    f"{path.relative_to(REPO_ROOT)}: {length} characters "
                    f"exceeds the App Store limit of {limit}."
                )


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--refresh-spec",
        action="store_true",
        help="Re-download Apple's OpenAPI spec, ignoring the local cache.",
    )
    args = parser.parse_args()

    try:
        spec = load_spec(refresh=args.refresh_spec)
    except Exception as e:
        print(f"error: failed to load OpenAPI spec: {e}", file=sys.stderr)
        return 2

    errors: list = []
    warnings: list = []

    lint_age_rating(spec, errors, warnings)
    lint_metadata_lengths(errors)

    for w in warnings:
        print(f"warning: {w}")
    for e in errors:
        print(f"error: {e}", file=sys.stderr)

    if errors:
        print(
            f"\nFAIL: {len(errors)} error(s), {len(warnings)} warning(s)",
            file=sys.stderr,
        )
        return 1
    print(f"OK: {len(warnings)} warning(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
