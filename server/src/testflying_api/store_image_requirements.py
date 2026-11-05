from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from struct import unpack


@dataclass(frozen=True)
class ImageInfo:
    width: int
    height: int
    format: str
    bit_depth: int | None = None
    color_type: int | None = None


@dataclass(frozen=True)
class StoreImageValidation:
    valid: bool
    message: str
    image: ImageInfo | None = None
    matched_label: str | None = None


APPLE_IPHONE_SIZES: tuple[tuple[int, int, str], ...] = (
    (1260, 2736, 'iPhone 6.9"'),
    (2736, 1260, 'iPhone 6.9" 横屏'),
    (1290, 2796, 'iPhone 6.9"'),
    (2796, 1290, 'iPhone 6.9" 横屏'),
    (1320, 2868, 'iPhone 6.9"'),
    (2868, 1320, 'iPhone 6.9" 横屏'),
    (1284, 2778, 'iPhone 6.5"'),
    (2778, 1284, 'iPhone 6.5" 横屏'),
    (1242, 2688, 'iPhone 6.5"'),
    (2688, 1242, 'iPhone 6.5" 横屏'),
    (1242, 2208, 'iPhone 5.5"'),
    (2208, 1242, 'iPhone 5.5" 横屏'),
)

APPLE_IPAD_SIZES: tuple[tuple[int, int, str], ...] = (
    (2064, 2752, 'iPad 13"'),
    (2752, 2064, 'iPad 13" 横屏'),
    (2048, 2732, 'iPad 12.9"'),
    (2732, 2048, 'iPad 12.9" 横屏'),
)

GOOGLE_SCREENSHOT_MIN_DIMENSION = 320
GOOGLE_SCREENSHOT_MAX_DIMENSION = 3840
GOOGLE_SCREENSHOT_MAX_RATIO = 2.0
GOOGLE_FEATURE_GRAPHIC_SIZE = (1024, 500)


def store_image_requirement(platform: str, slot_key: str) -> dict[str, object]:
    if platform == "android":
        if slot_key == "feature_graphic_url":
            return {
                "platform": "android",
                "slot": slot_key,
                "kind": "google_feature_graphic",
                "summary": "Google Play 功能宣传图必须是 1024 x 500，JPG 或 24-bit PNG。",
                "fallbackAspectRatio": _aspect_ratio(*GOOGLE_FEATURE_GRAPHIC_SIZE),
                "exactSizes": [
                    {
                        "width": GOOGLE_FEATURE_GRAPHIC_SIZE[0],
                        "height": GOOGLE_FEATURE_GRAPHIC_SIZE[1],
                        "label": "Feature graphic",
                    }
                ],
                "allowedFormats": ["jpeg", "png"],
                "pngNoAlpha": True,
            }
        return {
            "platform": "android",
            "slot": slot_key,
            "kind": "google_screenshot",
            "summary": "Google Play 截图最小边 320、最大边 3840，长边不能超过短边 2 倍。",
            "fallbackAspectRatio": _aspect_ratio(9, 16)
            if slot_key == "phone_screenshots"
            else _aspect_ratio(16, 9),
            "minDimension": GOOGLE_SCREENSHOT_MIN_DIMENSION,
            "maxDimension": GOOGLE_SCREENSHOT_MAX_DIMENSION,
            "maxAspectRatio": GOOGLE_SCREENSHOT_MAX_RATIO,
            "allowedFormats": ["jpeg", "png"],
            "pngNoAlpha": True,
        }

    sizes = APPLE_IPAD_SIZES if slot_key == "tablet_screenshots" else APPLE_IPHONE_SIZES
    return {
        "platform": "ios",
        "slot": slot_key,
        "kind": "apple_exact_sizes",
        "summary": "App Store Connect 截图必须匹配当前设备族的精确尺寸。",
        "fallbackAspectRatio": _aspect_ratio(sizes[0][0], sizes[0][1]),
        "exactSizes": [
            {"width": width, "height": height, "label": label}
            for width, height, label in sizes
        ],
        "allowedFormats": ["jpeg", "png"],
        "pngNoAlpha": False,
    }


def validate_store_image(
    *,
    platform: str,
    slot_key: str,
    filename: str,
    content_type: str,
    content: bytes,
) -> StoreImageValidation:
    requirement = store_image_requirement(platform, slot_key)
    image = parse_image_info(content)
    if image is None:
        return StoreImageValidation(
            False,
            f"{filename} 不是可识别的 PNG/JPEG 图片",
        )

    format_result = _validate_format(requirement, image, filename, content_type)
    if format_result is not None:
        return StoreImageValidation(False, format_result, image)

    kind = str(requirement["kind"])
    if kind == "apple_exact_sizes":
        for size in requirement.get("exactSizes", []):
            if image.width == size["width"] and image.height == size["height"]:
                return StoreImageValidation(
                    True,
                    f"符合 {size['label']} {image.width} x {image.height}",
                    image,
                    str(size["label"]),
                )
        return StoreImageValidation(
            False,
            f"Apple 要求精确尺寸，当前 {image.width} x {image.height} 不匹配",
            image,
        )

    if kind == "google_feature_graphic":
        width, height = GOOGLE_FEATURE_GRAPHIC_SIZE
        if image.width == width and image.height == height:
            return StoreImageValidation(
                True,
                f"符合 Google Play Feature graphic {width} x {height}",
                image,
                "Feature graphic",
            )
        return StoreImageValidation(
            False,
            f"Google Play 功能宣传图必须是 {width} x {height}，当前 {image.width} x {image.height}",
            image,
        )

    shortest = min(image.width, image.height)
    longest = max(image.width, image.height)
    if shortest < GOOGLE_SCREENSHOT_MIN_DIMENSION:
        message = (
            f"Google Play 截图最小边不能小于 {GOOGLE_SCREENSHOT_MIN_DIMENSION}px，"
            f"当前 {image.width} x {image.height}"
        )
        return StoreImageValidation(
            False,
            message,
            image,
        )
    if longest > GOOGLE_SCREENSHOT_MAX_DIMENSION:
        message = (
            f"Google Play 截图最大边不能超过 {GOOGLE_SCREENSHOT_MAX_DIMENSION}px，"
            f"当前 {image.width} x {image.height}"
        )
        return StoreImageValidation(
            False,
            message,
            image,
        )
    if longest / shortest > GOOGLE_SCREENSHOT_MAX_RATIO:
        return StoreImageValidation(
            False,
            f"Google Play 截图长边不能超过短边 2 倍，当前 {image.width} x {image.height}",
            image,
        )
    return StoreImageValidation(
        True,
        f"符合 Google Play 截图 {image.width} x {image.height}",
        image,
        "Google Play screenshot",
    )


def parse_image_info(content: bytes) -> ImageInfo | None:
    if content.startswith(b"\x89PNG\r\n\x1a\n"):
        return _parse_png_info(content)
    if content.startswith(b"\xff\xd8"):
        return _parse_jpeg_info(content)
    return None


def _parse_png_info(content: bytes) -> ImageInfo | None:
    if len(content) < 33 or content[12:16] != b"IHDR":
        return None
    width, height = unpack(">II", content[16:24])
    return ImageInfo(
        width=width,
        height=height,
        format="png",
        bit_depth=content[24],
        color_type=content[25],
    )


def _parse_jpeg_info(content: bytes) -> ImageInfo | None:
    index = 2
    while index + 4 < len(content):
        if content[index] != 0xFF:
            index += 1
            continue
        while index < len(content) and content[index] == 0xFF:
            index += 1
        if index >= len(content):
            return None
        marker = content[index]
        index += 1
        if marker in {0xD8, 0xD9}:
            continue
        if marker == 0xDA or index + 2 > len(content):
            return None
        length = unpack(">H", content[index : index + 2])[0]
        if length < 2 or index + length > len(content):
            return None
        if marker in {
            0xC0,
            0xC1,
            0xC2,
            0xC3,
            0xC5,
            0xC6,
            0xC7,
            0xC9,
            0xCA,
            0xCB,
            0xCD,
            0xCE,
            0xCF,
        }:
            if length < 7:
                return None
            height, width = unpack(">HH", content[index + 3 : index + 7])
            return ImageInfo(width=width, height=height, format="jpeg")
        index += length
    return None


def _validate_format(
    requirement: dict[str, object],
    image: ImageInfo,
    filename: str,
    content_type: str,
) -> str | None:
    allowed_formats = set(requirement.get("allowedFormats", []))
    extension = Path(filename).suffix.lower().lstrip(".")
    normalized_extension = "jpeg" if extension in {"jpg", "jpeg"} else extension
    normalized_content_type = content_type.lower().split(";", 1)[0].strip()
    content_format = {
        "image/jpeg": "jpeg",
        "image/jpg": "jpeg",
        "image/png": "png",
    }.get(normalized_content_type, "")
    if image.format not in allowed_formats:
        return "商店图片只支持 JPG 或 PNG"
    if normalized_extension and normalized_extension not in allowed_formats:
        return f"{filename} 的文件扩展名不符合商店图片格式要求"
    if content_format and content_format != image.format:
        return f"{filename} 的文件类型和图片内容不一致"
    if requirement.get("pngNoAlpha") and image.format == "png":
        if image.bit_depth != 8 or image.color_type != 2:
            return "Google Play 只接受 24-bit PNG，不支持透明通道"
    return None


def _aspect_ratio(width: int, height: int) -> str:
    return f"{width} / {height}"
