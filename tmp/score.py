import argparse
import os
import sys
import cv2
import numpy as np
import requests


def load_image(source):
    """Loads an image from a local path or a remote URL."""
    if source.startswith(("http://", "https://")):
        try:
            response = requests.get(source, timeout=10)
            response.raise_for_status()
            image_array = np.asarray(bytearray(response.content), dtype=np.uint8)
            img = cv2.imdecode(image_array, cv2.IMREAD_COLOR)
            if img is None:
                raise ValueError("Failed to decode image from URL.")
            return img
        except Exception as e:
            print(f"Error fetching URL: {e}")
            sys.exit(1)
    else:
        if not os.path.exists(source):
            print(f"Error: Local file '{source}' does not exist.")
            sys.exit(1)
        img = cv2.imread(source, cv2.IMREAD_COLOR)
        if img is None:
            print(f"Error: Failed to read local image '{source}'.")
            sys.exit(1)
        return img


def calculate_vector_smoothness(image):
    """Quantifies how 'vectorized' and smooth the boundaries of the image look.

    Returns a score where higher values mean smoother, cleaner lines.
    """
    # 1. Convert to grayscale
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)

    # 2. Isolate the linework/boundaries.
    # Since the images are cleanly quantized, Otsu's thresholding separates the regions perfectly.
    _, binary = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)

    # 3. Find contours (external and internal boundaries)
    contours, _ = cv2.findContours(
        binary, cv2.RETR_LIST, cv2.CHAIN_APPROX_NONE
    )

    total_original_points = 0
    total_simplified_points = 0
    valid_contours_count = 0

    # We use a small epsilon relative to standard resolution to catch micro-jaggies
    epsilon = 1.5

    for cnt in contours:
        # Ignore tiny speckles or noise artifacts (less than 15 pixels long)
        if len(cnt) < 15:
            continue

        valid_contours_count += 1
        total_original_points += len(cnt)

        # Apply Douglas-Peucker simplification
        simplified = cv2.approxPolyDP(cnt, epsilon, closed=True)
        total_simplified_points += len(simplified)

    if total_original_points == 0:
        return 0.0

    # 4. Calculate the Simplification Ratio
    # If an edge is already perfectly straight/smooth, it requires very few points to represent it.
    # Micro-jaggies (Image 1) resist simplification, keeping the ratio low.
    reduction_ratio = 1.0 - (total_simplified_points / total_original_points)

    # Scale to a percentage index (0 to 100)
    # A higher percentage means the image boundaries are cleaner and more vector-like.
    vector_score = min(max(reduction_ratio * 100, 0.0), 100.0)

    return vector_score


def main():
    parser = argparse.ArgumentParser(
        description="Quantify how 'vectorized' and smooth an image's edges look."
    )
    parser.add_argument(
        "source", help="Local file path or remote HTTP/HTTPS URL of the image"
    )
    args = parser.parse_args()

    # Load and process image
    img = load_image(args.source)
    score = calculate_vector_smoothness(img)

    print("\n" + "=" * 40)
    print(f" Image Smoothness Analysis")
    print("=" * 40)
    print(f"Source:       {args.source}")
    print(f"Vector Score: {score:.2f} / 100")
    print("-" * 40)

    if score < 45:
        print("Verdict:      Highly Aliased / Jagged (Low Vector Likeness)")
    elif score < 65:
        print("Verdict:      Moderately Smooth / Some Jagged Edges")
    else:
        print("Verdict:      Clean & Smooth (High Vector Likeness)")
    print("=" * 40 + "\n")


if __name__ == "__main__":
    main()
