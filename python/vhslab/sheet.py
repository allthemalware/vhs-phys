"""Contact sheet of labelled frames: sheet.py out.png label=path label=path ..."""
import sys
from PIL import Image, ImageDraw

def main(out, items, cols=3, w=480):
    ims = []
    for it in items:
        label, path = it.split("=", 1)
        im = Image.open(path).convert("RGB")
        h = int(im.height * w / im.width)
        im = im.resize((w, h), Image.LANCZOS)
        d = ImageDraw.Draw(im)
        d.rectangle([0, 0, 8 + 7 * len(label), 16], fill=(0, 0, 0))
        d.text((4, 2), label, fill=(255, 255, 0))
        ims.append(im)
    rows = (len(ims) + cols - 1) // cols
    h = ims[0].height
    sheet = Image.new("RGB", (cols * w, rows * h))
    for i, im in enumerate(ims):
        sheet.paste(im, ((i % cols) * w, (i // cols) * h))
    sheet.save(out, quality=88) if out.lower().endswith((".jpg", ".jpeg")) else sheet.save(out)

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
