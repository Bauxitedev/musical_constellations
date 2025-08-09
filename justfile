# On Windows use `just --shell bash.exe notify "hi"` (that uses WSL.exe btw)

# Run this to see what's going to be actually pushed when you run `push-new-tag`
push-new-tag-dry-run:
    @echo "The following changes will be made to your tags:"
    git push --tags --dry-run

# Creates a new tag and pushes it to GitHub. This will trigger a new release automatically.
push-new-tag tag:
    git tag -a {{tag}} -m "auto pushed" && git push --follow-tags

# Run this if you're deleted tags on GitHub to delete them locally as well. This prevents pushing them again.
[confirm("Are you sure you want to prune the tags locally?")]
prune-tags:
    git fetch --prune --prune-tags
    @echo "Done"