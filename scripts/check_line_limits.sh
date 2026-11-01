#!/bin/sh
# 仓库规范门禁（README「仓库规范」）：
#   - 迭代中的源码文件 <= 800 行（硬上限，对全部 .rs/.c/.cpp/.h 生效）
#   - 相对基线新增的文件 <= 400 行（默认基线 HEAD；CI 传 PR base sha）
# 用法: check_line_limits.sh [base_ref]
set -eu
fail=0

# 1) 硬上限 800 行
while IFS= read -r f; do
	lines=$(wc -l < "$f")
	if [ "$lines" -gt 800 ]; then
		echo "OVER-800: $f ($lines lines)"
		fail=1
	fi
done <<FILELIST
$(find crates -type f \( -name '*.rs' -o -name '*.cpp' -o -name '*.h' -o -name '*.c' \) ! -path '*/build/*')
FILELIST

# 2) 新增文件 <= 400 行
base="${1:-HEAD}"
if git rev-parse --verify -q "$base" >/dev/null 2>&1; then
	git diff --name-only --diff-filter=A "$base" -- 2>/dev/null |
	grep -E '\.(rs|cpp|h|c)$' |
	while IFS= read -r f; do
		[ -f "$f" ] || continue
		lines=$(wc -l < "$f")
		if [ "$lines" -gt 400 ]; then
			echo "OVER-400(new vs $base): $f ($lines lines)"
			fail=1
		fi
	done
else
	echo "note: base ref '$base' not found, skip new-file check"
fi

[ "$fail" -eq 0 ] && echo "line limits OK"
exit "$fail"
