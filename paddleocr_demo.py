#!/usr/bin/env python3
import sys
import os
import math
import requests
import argparse
from io import BytesIO
from PIL import Image, ImageDraw, ImageFont

# Suppress PaddleOCR/PaddlePaddle verbose log messages if possible
os.environ["PPOCR_LOG_LEVEL"] = "WARNING"

def download_image(url):
    """Downloads an image from a URL and returns it as a PIL Image."""
    print(f"# Downloading image from {url}...", file=sys.stderr)
    response = requests.get(url, timeout=30)
    response.raise_for_status()
    return Image.open(BytesIO(response.content))

def is_valid_text(text, confidence, min_confidence=0.7, min_ascii_ratio=0.7):
    """Filters out low confidence text or text containing mostly non-ASCII glyphs."""
    if confidence < min_confidence:
        return False
    if not text:
        return False
    # Count printable ASCII characters
    ascii_count = sum(1 for c in text if 32 <= ord(c) <= 126)
    return (ascii_count / len(text)) >= min_ascii_ratio

def estimate_text_color(img_rgb, bbox, global_bg_color):
    """Estimates the color of the text in a bounding box by finding pixels most different from the background."""
    x0 = max(0, int(round(min(p[0] for p in bbox))))
    x1 = min(img_rgb.width, int(round(max(p[0] for p in bbox))))
    y0 = max(0, int(round(min(p[1] for p in bbox))))
    y1 = min(img_rgb.height, int(round(max(p[1] for p in bbox))))

    if x1 <= x0 or y1 <= y0:
        return (0, 0, 0)

    cropped = img_rgb.crop((x0, y0, x1, y1))
    
    import numpy as np
    cropped_arr = np.array(cropped)
    pixels = cropped_arr.reshape(-1, 3)
    
    if len(pixels) == 0:
        return (0, 0, 0)

    # Estimate local background as the median color of all pixels in the cropped box.
    # Since background always occupies the majority of the bounding box area, the median
    # color is a highly robust estimation of the background color.
    r_vals = sorted(p[0] for p in pixels)
    g_vals = sorted(p[1] for p in pixels)
    b_vals = sorted(p[2] for p in pixels)
    n = len(pixels)
    local_bg = (int(r_vals[n//2]), int(g_vals[n//2]), int(b_vals[n//2]))

    # Find the 15% of pixels that differ most from the local background color
    distances = []
    for p in pixels:
        p_tuple = (int(p[0]), int(p[1]), int(p[2]))
        dist = math.sqrt((p_tuple[0] - local_bg[0])**2 + (p_tuple[1] - local_bg[1])**2 + (p_tuple[2] - local_bg[2])**2)
        distances.append((dist, p_tuple))

    distances.sort(key=lambda x: x[0], reverse=True)

    top_n = max(1, int(len(distances) * 0.15))
    text_pixels = [p for dist, p in distances[:top_n]]

    avg_r = sum(p[0] for p in text_pixels) // len(text_pixels)
    avg_g = sum(p[1] for p in text_pixels) // len(text_pixels)
    avg_b = sum(p[2] for p in text_pixels) // len(text_pixels)

    return (avg_r, avg_g, avg_b)

def recreate_image(img, results, output_filename="recreated.png"):
    img_rgb = img.convert("RGB")
    global_bg_color = img_rgb.getpixel((0, 0))

    # Create a blank image of the same size with background color
    recreated_img = Image.new("RGB", img.size, global_bg_color)
    draw = ImageDraw.Draw(recreated_img)

    font_path = "/usr/share/fonts/noto/NotoSans-Regular.ttf"

    res = results[0]
    texts = res.get('rec_texts', [])
    polys = res.get('rec_polys', [])
    scores = res.get('rec_scores', [])

    for text, bbox, confidence in zip(texts, polys, scores):
        if not is_valid_text(text, confidence):
            continue

        x0, y0 = bbox[0]
        x1, y1 = bbox[1]
        x2, y2 = bbox[2]
        x3, y3 = bbox[3]

        left_height = math.sqrt((x3 - x0)**2 + (y3 - y0)**2)
        right_height = math.sqrt((x2 - x1)**2 + (y2 - y1)**2)
        height = int(round((left_height + right_height) / 2.0))

        x = int(round(x0))
        y = int(round(y0))

        # Estimate box width and height
        box_width = max(1, int(round(max(x0, x1, x2, x3) - min(x0, x1, x2, x3))))
        box_height = max(1, int(round(max(y0, y1, y2, y3) - min(y0, y1, y2, y3))))

        # Determine color
        color = estimate_text_color(img_rgb, bbox, global_bg_color)

        # Scale down font size until the text fits in the bounding box
        font_size = height
        font = None
        while font_size > 4:
            try:
                temp_font = ImageFont.truetype(font_path, font_size)
            except Exception:
                temp_font = ImageFont.load_default()
                font = temp_font
                break

            # Check bounding box of the text using the current font
            left, top, right, bottom = temp_font.getbbox(text)
            text_width = right - left
            text_height = bottom - top

            if text_width <= box_width and text_height <= box_height:
                font = temp_font
                break
            font_size -= 1

        if font is None:
            font = ImageFont.load_default()

        draw.text((x, y), text, fill=color, font=font)

    # Save the recreated image
    recreated_img.save(output_filename)
    print(f"\n# Recreated image saved to {output_filename}", file=sys.stderr)

def draw_bounding_boxes(img, results, output_filename="boxes.png"):
    """Saves a copy of the input image with red outlines around the detected text blocks."""
    img_with_boxes = img.convert("RGB")
    draw = ImageDraw.Draw(img_with_boxes)
    
    res = results[0]
    texts = res.get('rec_texts', [])
    polys = res.get('rec_polys', [])
    scores = res.get('rec_scores', [])

    for text, bbox, confidence in zip(texts, polys, scores):
        if not is_valid_text(text, confidence):
            continue

        # Convert bbox points to tuple format for ImageDraw
        polygon_points = [(p[0], p[1]) for p in bbox]
        # Draw red polygon outline with width=2
        draw.polygon(polygon_points, outline=(255, 0, 0), width=2)

    img_with_boxes.save(output_filename)
    print(f"# Bounding boxes image saved to {output_filename}", file=sys.stderr)

def main():
    parser = argparse.ArgumentParser(description="PaddleOCR Demo that recreates images and draws bounding boxes.")
    parser.add_argument("url_or_path", nargs="?", default="https://pcp.ovh/Sman.png",
                        help="URL or local path of the input image (default: https://pcp.ovh/Sman.png)")
    parser.add_argument("--show-boxes", "-b", action="store_true",
                        help="Save a copy of the input image with bounding boxes drawn as boxes.png")
    parser.add_argument("--model", "-m", choices=["tiny", "small", "medium"], default="medium",
                        help="PaddleOCR model size to use: tiny, small, or medium (default: medium)")
    args = parser.parse_args()

    # Load image
    url_or_path = args.url_or_path
    if url_or_path.startswith("http://") or url_or_path.startswith("https://"):
        try:
            img = download_image(url_or_path)
        except Exception as e:
            print(f"Error downloading image: {e}", file=sys.stderr)
            sys.exit(1)
    else:
        if not os.path.exists(url_or_path):
            print(f"Error: File {url_or_path} does not exist.", file=sys.stderr)
            sys.exit(1)
        img = Image.open(url_or_path)

    # Save to a temporary file since paddleocr expects a file path or numpy array
    temp_filename = "temp_ocr_image.png"
    img.save(temp_filename)

    try:
        from paddleocr import PaddleOCR
        # Initialize PaddleOCR
        ocr = PaddleOCR(
            text_detection_model_name=f"PP-OCRv6_{args.model}_det",
            text_recognition_model_name=f"PP-OCRv6_{args.model}_rec",
            use_textline_orientation=True,
            lang='en',
            enable_mkldnn=False,
            use_doc_orientation_classify=False,
            use_doc_unwarping=False
        )
        
        print(f"# Running OCR...", file=sys.stderr)
        results = list(ocr.predict(temp_filename))
    finally:
        if os.path.exists(temp_filename):
            os.remove(temp_filename)

    if not results or 'rec_texts' not in results[0]:
        print("# No text detected.", file=sys.stderr)
        return

    print("# Generated draw commands to recreate the image:")
    res = results[0]
    img_rgb = img.convert("RGB")
    global_bg_color = img_rgb.getpixel((0, 0))

    texts = res.get('rec_texts', [])
    polys = res.get('rec_polys', [])
    scores = res.get('rec_scores', [])

    for text, bbox, confidence in zip(texts, polys, scores):
        if not is_valid_text(text, confidence):
            continue

        x0, y0 = bbox[0]
        x1, y1 = bbox[1]
        x2, y2 = bbox[2]
        x3, y3 = bbox[3]

        left_height = math.sqrt((x3 - x0)**2 + (y3 - y0)**2)
        right_height = math.sqrt((x2 - x1)**2 + (y2 - y1)**2)
        height = int(round((left_height + right_height) / 2.0))

        x = int(round(x0))
        y = int(round(y0))

        color = estimate_text_color(img_rgb, bbox, global_bg_color)

        escaped_text = text.replace('"', '\\"')
        print(f'draw_text(text="{escaped_text}", x={x}, y={y}, height={height}, color={color})')

    # Recreate the image based on the OCR results
    recreate_image(img, results)

    # Save bounding boxes if requested
    if args.show_boxes:
        draw_bounding_boxes(img, results)

if __name__ == "__main__":
    main()
