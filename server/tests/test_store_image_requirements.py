from __future__ import annotations

from testflying_api.store_image_requirements import parse_image_info, validate_store_image
from tests.fixtures import make_jpeg_header_bytes, make_png_header_bytes


def test_parse_png_dimensions() -> None:
    image = parse_image_info(make_png_header_bytes(1290, 2796))

    assert image is not None
    assert image.width == 1290
    assert image.height == 2796
    assert image.format == "png"
    assert image.bit_depth == 8
    assert image.color_type == 2


def test_parse_jpeg_dimensions() -> None:
    image = parse_image_info(make_jpeg_header_bytes(1024, 500))

    assert image is not None
    assert image.width == 1024
    assert image.height == 500
    assert image.format == "jpeg"


def test_apple_phone_screenshot_requires_exact_size() -> None:
    valid = validate_store_image(
        platform="ios",
        slot_key="phone_screenshots",
        filename="iphone.png",
        content_type="image/png",
        content=make_png_header_bytes(1290, 2796),
    )
    invalid = validate_store_image(
        platform="ios",
        slot_key="phone_screenshots",
        filename="iphone.png",
        content_type="image/png",
        content=make_png_header_bytes(1080, 2400),
    )

    assert valid.valid
    assert valid.matched_label == 'iPhone 6.9"'
    assert not invalid.valid
    assert "Apple 要求精确尺寸" in invalid.message


def test_apple_tablet_screenshot_requires_ipad_size() -> None:
    valid = validate_store_image(
        platform="ios",
        slot_key="tablet_screenshots",
        filename="ipad.jpg",
        content_type="image/jpeg",
        content=make_jpeg_header_bytes(2048, 2732),
    )

    assert valid.valid
    assert valid.matched_label == 'iPad 12.9"'


def test_google_screenshot_requires_bounds_and_ratio() -> None:
    valid = validate_store_image(
        platform="android",
        slot_key="phone_screenshots",
        filename="phone.png",
        content_type="image/png",
        content=make_png_header_bytes(1080, 1920),
    )
    too_small = validate_store_image(
        platform="android",
        slot_key="phone_screenshots",
        filename="phone.png",
        content_type="image/png",
        content=make_png_header_bytes(300, 533),
    )
    too_long = validate_store_image(
        platform="android",
        slot_key="phone_screenshots",
        filename="phone.png",
        content_type="image/png",
        content=make_png_header_bytes(400, 1000),
    )

    assert valid.valid
    assert not too_small.valid
    assert "最小边不能小于" in too_small.message
    assert not too_long.valid
    assert "长边不能超过短边 2 倍" in too_long.message


def test_google_feature_graphic_requires_exact_size() -> None:
    valid = validate_store_image(
        platform="android",
        slot_key="feature_graphic_url",
        filename="feature.png",
        content_type="image/png",
        content=make_png_header_bytes(1024, 500),
    )
    invalid = validate_store_image(
        platform="android",
        slot_key="feature_graphic_url",
        filename="feature.png",
        content_type="image/png",
        content=make_png_header_bytes(1200, 500),
    )

    assert valid.valid
    assert valid.matched_label == "Feature graphic"
    assert not invalid.valid
    assert "1024 x 500" in invalid.message


def test_google_png_rejects_alpha_channel() -> None:
    result = validate_store_image(
        platform="android",
        slot_key="phone_screenshots",
        filename="phone.png",
        content_type="image/png",
        content=make_png_header_bytes(1080, 1920, color_type=6),
    )

    assert not result.valid
    assert "24-bit PNG" in result.message
