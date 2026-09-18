# img2irc for Python

```python
from img2irc import Simple

output = Simple(width=80, mode="ansi24").generate("photo.png")
output.write("art.ans")
output.save_preview("preview.png")
```

see [Python documentation](../docs/python.md) for the APIs and wheel builds.
