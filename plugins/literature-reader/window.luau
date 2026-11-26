--!nocheck
-- Window half: palette commands. The reading view itself is a host block.
local tern = require("tern")

local BLOCK = "literature-reader.reader"

-- PDFs open in the reader from the Files pane, the palette, drag-and-drop,
-- links and `tern open`; the built-in PDF block is not the primary view.
tern.route.open(function(req, _cx)
	if type(req.path) == "string" and string.match(string.lower(req.path), "%.pdf$") then
		return { block = BLOCK, args = { req.path } }
	end
	return nil
end)

tern.command({
	id = "open",
	title = "文献阅读器：打开标签页",
	icon = "newspaper",
	run = function(cx)
		local pane = cx:new_block(BLOCK, tern.json.array(), "tab")
		if not pane then
			cx:toast("error", "无法打开文献阅读器", "确认插件已启用，且会话主机支持插件区块")
		end
	end,
})

tern.command({
	id = "open-path",
	title = "文献阅读器：打开 PDF…",
	icon = "file-pdf",
	run = function(cx)
		cx:prompt({ placeholder = "PDF 路径", icon = "file-pdf", action = "打开" })
			:next(function(path, _err, next_cx)
				if path and path ~= "" and next_cx then
					next_cx:new_block(BLOCK, { path }, "tab")
				end
			end)
	end,
})

tern.command({
	id = "login",
	title = "文献阅读器：登录 CodeBuddy",
	icon = "log-in",
	run = function(cx)
		tern.kv.set("login_requested", true)
		local pane = cx:new_block(BLOCK, tern.json.array(), "tab")
		if not pane then
			tern.kv.set("login_requested", nil)
			cx:toast("error", "无法打开文献阅读器", "确认插件已启用，且会话主机支持插件区块")
		end
	end,
})

tern.command({
	id = "set-exe",
	title = "文献阅读器：设置可执行文件…",
	icon = "terminal",
	run = function(cx)
		local current = tern.kv.get("exe")
		cx:prompt({
			value = type(current) == "string" and current or "",
			placeholder = "literature-reader 可执行文件的完整路径",
			icon = "terminal",
			action = "设置",
		}):next(function(path, _err, next_cx)
			if not path or not next_cx then
				return
			end
			path = string.gsub(path, "^%s+", "")
			path = string.gsub(path, "%s+$", "")
			if path == "" then
				tern.kv.set("exe", nil)
				next_cx:toast("info", "已清除自定义可执行文件路径")
				return
			end
			if not tern.fs.exists(path) then
				next_cx:toast("error", "文件不存在", path)
				return
			end
			tern.kv.set("exe", path)
			next_cx:toast("success", "已设置文献阅读器后端", path)
		end)
	end,
})

tern.command({
	id = "toggle-auto",
	title = "文献阅读器：自动翻译开/关",
	icon = "sparkle",
	run = function(cx)
		local enabled = tern.kv.get("auto_translate") == true
		tern.kv.set("auto_translate", not enabled)
		cx:toast("info", enabled and "已关闭自动翻译" or "已开启自动翻译")
	end,
})
