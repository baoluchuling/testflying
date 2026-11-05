from __future__ import annotations

import base64
import plistlib
import struct
from io import BytesIO
from zipfile import ZipFile

ANDROID_APK_BASE64 = (
    "UEsDBBQAAgAIAABAIQBUszT96wEAAMQEAAATAAAAQW5kcm9pZE1hbmlmZXN0LnhtbJWSzW7TQBSFzzgNcdSUulDE"
    "XxesEEJqIqB0wQ7YIcQGCSHEJrQpWLSu5biIXXkEnoEHQDwET4AQa9YseAX45nqMjdtKYOfkjs/9O3Nneor1eUFy"
    "WtPXSDqn5slb60tgAzwD78FH8AV8Bz99gJOegucgAx/AN/ADXKDuVTAGm+ABWNQbzVRorlT7ynSf/22Yo55HmmrP"
    "PCvagtlDV6pdmMdkvNaTv6Kli/8QVXfL/tRewqZ8H18zUUlkoZfw5QkxfbpN9QJ2175KvWJdVR/gycgqiE2x0nW8"
    "JW+uO5rwzlFdxU9Zjzvx47CnCXxO7wmeGXGTI3Vl3XL4LeKmppgTgPHqSu0QWVDpng5sPtsnTOZ/cpoTik1/Br9j"
    "+kr5G1VpH8O8tcjczsXv8YCIfVPrq81sbht4boNNviLd0E2zt8zGZMyt8jrwp+Dvi5+Jr5nSyeutT+Sdi3UNezly"
    "bg0UfefmYMQ6B078PFzCbu0K6xdPjB36rvCfWrx/TrNe5R2EWQ+DDXnJXVdxzm6NV6xeUtdrxV0JXM/udRW3EvyL"
    "2OXALQduhD1TcZbruaUmNxkFvQ9tho3es0Fv1NK70NJxPnD9jrZemEm3Vt3jVItfPabHoKnHKHqHw1ZOxUWHdY9u"
    "rZofdObuOnx9Tr8BUEsDBBQAAAAAAABAIQA8uqfn4AIAAOACAAAOAAAAcmVzb3VyY2VzLmFyc2MCAAwA4AIAAAEA"
    "AAABABwAMAAAAAEAAAAAAAAAAAEAACAAAAAAAAAAAAAAAAsLQXV0byBQYXJzZWQAAAAAAiABpAIAAH8AAABjAG8A"
    "bQAuAGUAeABhAG0AcABsAGUALgBhAHUAdABvAHAAYQByAHMAZQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIAEAAAMAAABgAQAA"
    "AgAAAAAAAAABABwAQAAAAAMAAAAAAAAAAAEAACgAAAAAAAAAAAAAAAcAAAAQAAAABARhdHRyAAYGc3RyaW5nAAUF"
    "c3R5bGUAAQAcADwAAAACAAAAAAAAAAABAAAkAAAAAAAAAAAAAAALAAAACAhhcHBfbmFtZQAICEFwcFRoZW1lAAAA"
    "AgIQABAAAAABAAAAAAAAAAICEAAUAAAAAgAAAAEAAAAAAAAAAQJUAGgAAAACAAAAAQAAAFgAAABAAAAAAAAAAAAA"
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAAAAAAAAA"
    "CAAAAwAAAAACAhAAFAAAAAMAAAABAAAAAAAAAAECVABoAAAAAwAAAAEAAABYAAAAQAAAAAAAAAAAAAAAAAAAAAAA"
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAEAAQAAAEECAwEAAAAA"
    "UEsBAhcDFAACAAgAAEAhAFSzNP3rAQAAxAQAABMAAAAAAAAAAAAgALaBAAAAAEFuZHJvaWRNYW5pZmVzdC54bWxQ"
    "SwECFwMUAAAAAAAAQCEAPLqn5+ACAADgAgAADgAAAAAAAAAAACAAtoEcAgAAcmVzb3VyY2VzLmFyc2NQSwUGAAAA"
    "AAIAAgB9AAAAKAUAAAAA"
)


def make_android_apk_bytes() -> bytes:
    return base64.b64decode(ANDROID_APK_BASE64)


def make_png_header_bytes(
    width: int,
    height: int,
    *,
    bit_depth: int = 8,
    color_type: int = 2,
) -> bytes:
    return (
        b"\x89PNG\r\n\x1a\n"
        + struct.pack(">I", 13)
        + b"IHDR"
        + struct.pack(">II", width, height)
        + bytes([bit_depth, color_type, 0, 0, 0])
        + b"\x00\x00\x00\x00"
    )


def make_jpeg_header_bytes(width: int, height: int) -> bytes:
    return (
        b"\xff\xd8"
        + b"\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00"
        + b"\xff\xc0"
        + struct.pack(">H", 17)
        + b"\x08"
        + struct.pack(">HH", height, width)
        + b"\x03\x01\x11\x00\x02\x11\x00\x03\x11\x00"
        + b"\xff\xd9"
    )


def make_ipa_bytes(
    *,
    bundle_identifier: str = "com.example.aurora",
    app_name: str = "Aurora",
    version: str = "1.2.3",
    build_number: str = "45",
) -> bytes:
    plist = plistlib.dumps(
        {
            "CFBundleIdentifier": bundle_identifier,
            "CFBundleDisplayName": app_name,
            "CFBundleShortVersionString": version,
            "CFBundleVersion": build_number,
        }
    )
    buffer = BytesIO()
    with ZipFile(buffer, "w") as archive:
        archive.writestr("Payload/Aurora.app/Info.plist", plist)
    return buffer.getvalue()
