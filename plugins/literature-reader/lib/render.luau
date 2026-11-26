--!nocheck
-- Every node the block draws. Pure: state in, Tern nodes out.
local tern = require("tern")
local util = require("./util")

local ui = tern.ui
local el = ui.el

local M = {}

local function textNode(class, value, math)
	if type(value) ~= "string" or value == "" then
		return nil
	end
	if math and #math > 0 then
		local spans = M.mathSpans(value, math)
		if spans then
			return el("p", { class = class }, { ui.text(spans) })
		end
	end
	return el("p", { class = class, text = value })
end

-- Markdown would read these as structure; escape them in plain runs.
local function escapeMarkdown(text)
	local out = string.gsub(text, "\\", "\\\\")
	out = string.gsub(out, "([`*_%[%]<>#|~$])", "\\%1")
	return out
end

local function runsMarkdown(runs)
	local parts = {}
	for _, run in ipairs(runs) do
		if type(run) == "table" then
			if type(run.tex) == "string" and run.tex ~= "" then
				table.insert(parts, "$" .. run.tex .. "$")
			elseif type(run.text) == "string" and run.text ~= "" then
				table.insert(parts, escapeMarkdown(run.text))
			end
		end
	end
	return table.concat(parts)
end

-- Prose with inline math goes through Markdown, whose $…$ runs are typeset
-- natively (the same TeX engine as display equations); everything else stays
-- a plain paragraph so a stray * or _ is never read as structure.
local function richNode(class, value, math, runs)
	if type(runs) == "table" and #runs > 0 then
		local markdown = runsMarkdown(runs)
		if markdown ~= "" then
			return el("div", { class = class .. " lr-md" }, { ui.md(markdown) })
		end
	end
	return textNode(class, value, math)
end

local function add(list, node)
	if node then
		table.insert(list, node)
	end
end

-- Split a translated string at the backend's byte ranges. Ranges are
-- { start, length } with 0-based byte offsets, exactly as string.sub needs.
function M.mathSpans(value, ranges)
	if type(value) ~= "string" or type(ranges) ~= "table" then
		return nil
	end
	local spans = {}
	local position = 0
	local any = false
	for _, range in ipairs(ranges) do
		if type(range) == "table" and type(range[1]) == "number" and type(range[2]) == "number" then
			local start = math.floor(range[1])
			local length = math.floor(range[2])
			if start >= 0 and length > 0 and start >= position and start < #value then
				if start > position then
					table.insert(spans, string.sub(value, position + 1, start))
				end
				local formula = string.sub(value, start + 1, start + length)
				if formula ~= "" then
					table.insert(spans, ui.span(formula, "mono accent"))
					any = true
				end
				position = start + length
			end
		end
	end
	if position < #value then
		table.insert(spans, string.sub(value, position + 1))
	end
	if not any or #spans == 0 then
		return nil
	end
	return spans
end

local function currentModelId(state)
	local snapshot = state.snapshot
	if not snapshot then
		return nil
	end
	return snapshot.model_id or snapshot.preferred_model
end

local function modelName(state)
	local id = currentModelId(state)
	if not id then
		return nil
	end
	local models = state.models and state.models.models
	if models then
		for _, model in ipairs(models) do
			if model.id == id then
				return model.name or model.id
			end
		end
	end
	return id
end

local function pageCount(state)
	local snapshot = state.snapshot
	return snapshot and snapshot.pages or 0
end

local function addTitle(out, state)
	local snapshot = state.snapshot
	local titleBlock = nil
	for _, block in ipairs(snapshot.blocks or {}) do
		if block.kind == "title" then
			titleBlock = block
			break
		end
	end
	local title = snapshot.title
	if type(title) ~= "string" or title == "" then
		title = titleBlock and titleBlock.text or ""
	end
	table.insert(out, el("h1", { class = "lr-title", text = title }))
	if state.showZh and titleBlock and titleBlock.translation and titleBlock.translation ~= "" then
		add(out, richNode("lr-title-zh lr-zh", titleBlock.translation, titleBlock.math, titleBlock.runs_zh))
	end
end

local function addHeading(out, state, block)
	add(out, richNode("lr-heading", block.text, nil, block.runs))
	if state.showZh and block.translation and block.translation ~= "" then
		add(out, richNode("lr-heading-zh lr-zh", block.translation, block.math, block.runs_zh))
	end
end

local function addParagraph(out, state, block, extra)
	local class = extra and ("lr-body " .. extra) or "lr-body"
	add(out, richNode(class, block.text, nil, block.runs))
	if state.showZh and block.translation and block.translation ~= "" then
		local zhClass = extra and ("lr-zh " .. extra .. "-zh") or "lr-zh"
		add(out, richNode(zhClass, block.translation, block.math, block.runs_zh))
	end
end

local function addCaption(out, state, block)
	add(out, richNode("lr-caption", block.text, nil, block.runs))
	if state.showZh and block.translation and block.translation ~= "" then
		add(out, richNode("lr-caption lr-zh", block.translation, block.math, block.runs_zh))
	end
end

local function addEquation(out, state, block)
	if type(block.tex) == "string" and block.tex ~= "" then
		table.insert(out, el("div", { class = "lr-equation" }, {
			ui.node("math", { text = block.tex, display = true }),
		}))
	else
		table.insert(out, el("pre", { class = "lr-equation", text = block.text }))
	end
	if state.showZh and block.translation and block.translation ~= "" then
		add(out, richNode("lr-zh lr-equation-zh", block.translation, block.math, block.runs_zh))
	end
end

local function addReference(out, state, block)
	add(out, richNode("lr-reference", block.text, nil, block.runs))
	if state.showZh and block.translation and block.translation ~= "" then
		add(out, richNode("lr-reference lr-zh", block.translation, block.math, block.runs_zh))
	end
end

local function addAsset(out, state, block, byIndex, consumed)
	local failed = state.imageFailed and state.imageFailed[block.index]
	local alt
	if type(block.text) == "string" and block.text ~= "" then
		alt = block.text
	else
		alt = block.kind == "table" and "表格" or "图"
	end
	local nodes = {}
	local blob = state.images and state.images[block.index]
	if blob then
		table.insert(nodes, ui.node("image", {
			key = "asset-image-" .. tostring(block.index),
			blob = blob,
			alt = alt,
			role = "figure",
		}))
	else
		table.insert(nodes, el("div", {
			key = "asset-missing-" .. tostring(block.index),
			class = "lr-figure-missing",
			text = failed and "图像无法显示（可尝试刷新）" or "正在生成图像…",
		}))
	end
	local captionIndex = block.caption
	if type(captionIndex) == "number" then
		local caption = byIndex[captionIndex]
		if caption then
			add(nodes, richNode("lr-caption", caption.text, nil, caption.runs))
			if state.showZh and caption.translation and caption.translation ~= "" then
				add(nodes, richNode("lr-caption lr-zh", caption.translation, caption.math, caption.runs_zh))
			end
			consumed[captionIndex] = true
		end
	end
	table.insert(out, el("div", {
		key = "asset-" .. tostring(block.index),
		class = "lr-asset",
	}, nodes))
end

-- A list group: all originals first, then all translations, never duplicated.
local function addGroup(out, state, blocks, first, last)
	for i = first, last do
		local block = blocks[i]
		if block then
			add(out, richNode("lr-body lr-item", block.text, nil, block.runs))
		end
	end
	if state.showZh then
		for i = first, last do
			local block = blocks[i]
			if block and block.translation and block.translation ~= "" then
				add(out, richNode("lr-zh lr-item-zh", block.translation, block.math, block.runs_zh))
			end
		end
	end
end

function M.document(state)
	local snapshot = state.snapshot
	local out = {}
	addTitle(out, state)

	local blocks = snapshot.blocks or {}
	local byIndex = {}
	local position = {}
	for i, block in ipairs(blocks) do
		if type(block.index) == "number" then
			byIndex[block.index] = block
			position[block.index] = i
		end
	end

	local groupEnd = {}
	local function boundary(value)
		local direct = position[value]
		if direct then
			return direct
		end
		for index, block in ipairs(blocks) do
			if type(block.index) == "number" and block.index >= value then
				return index
			end
		end
		return #blocks + 1
	end
	for _, group in ipairs(snapshot.groups or {}) do
		local from = position[group.start] or boundary(group.start)
		local last = boundary(group["end"])
		local startBlock = blocks[from]
		if startBlock and last > from then
			groupEnd[startBlock.index] = last
		end
	end

	local consumed = {}
	-- A caption paired to a figure may come before it; the asset draws it.
	local paired = {}
	for _, block in ipairs(blocks) do
		if (block.kind == "figure" or block.kind == "table") and type(block.caption) == "number" then
			paired[block.caption] = true
		end
	end
	local i = 1
	while i <= #blocks do
		local block = blocks[i]
		if consumed[block.index] then
			i = i + 1
		elseif block.kind == "title" then
			i = i + 1
		elseif groupEnd[block.index] then
			addGroup(out, state, blocks, i, groupEnd[block.index] - 1)
			i = groupEnd[block.index]
		elseif block.kind == "author" then
			add(out, textNode("lr-author", block.text))
			i = i + 1
		elseif block.kind == "heading" then
			addHeading(out, state, block)
			i = i + 1
		elseif block.kind == "equation" then
			addEquation(out, state, block)
			i = i + 1
		elseif block.kind == "figure" or block.kind == "table" then
			addAsset(out, state, block, byIndex, consumed)
			i = i + 1
		elseif block.kind == "caption" then
			if not paired[block.index] then
				addCaption(out, state, block)
			end
			i = i + 1
		elseif block.kind == "reference_heading" then
			table.insert(out, el("h2", { class = "lr-heading lr-reference-heading", text = block.text }))
			i = i + 1
		elseif block.kind == "reference" then
			addReference(out, state, block)
			i = i + 1
		else
			addParagraph(out, state, block)
			i = i + 1
		end
	end

	local notices = snapshot.notices
	if type(notices) == "table" and #notices > 0 then
		local lines = {}
		for _, notice in ipairs(notices) do
			if type(notice) == "string" and notice ~= "" then
				table.insert(lines, notice)
			end
		end
		if #lines > 0 then
			add(out, el("p", { class = "lr-notices", text = table.concat(lines, " · ") }))
		end
	end

	return ui.col({ el("div", { class = "lr-doc" }, out) })
end

local function documentItems(documents)
	local items = {}
	for i, document in ipairs(documents or {}) do
		if type(document.path) == "string" then
			table.insert(items, ui.node("item", {
				key = tostring(i),
				label = document.title or util.basename(document.path),
				detail = document.path,
				icon = "file-pdf",
				actions = { click = "open=" .. document.path },
			}))
		end
	end
	return items
end

local function itemsNode(items, key)
	if #items == 0 then
		return nil
	end
	local listNode = ui.node("list", { key = key }, items)
	if #items > 8 then
		return ui.node("scroll", { key = key .. "-scroll", max = { h = "8lines" }, bar = "auto" }, { listNode })
	end
	return listNode
end

function M.openCard(state)
	local children = {}
	add(children, el("p", { class = "lr-hint", text = "最近阅读" }))
	local recent = state.recent and state.recent.documents
	if state.recentLoading and not recent then
		add(children, el("p", { class = "lr-hint", text = "正在读取最近文献…" }))
	elseif state.recentError then
		add(children, el("p", { class = "lr-error-text", text = state.recentError }))
	end
	local recentItems = documentItems(recent)
	add(children, itemsNode(recentItems, "open-recent"))
	if #recentItems == 0 and not state.recentLoading then
		add(children, el("p", { class = "lr-hint", text = "还没有最近阅读的文献。" }))
	end

	local folder = state.folderPdfs or {}
	if #folder > 0 then
		add(children, el("p", { class = "lr-hint", text = "同目录的 PDF" }))
		local items = {}
		for i, path in ipairs(folder) do
			table.insert(items, ui.node("item", {
				key = tostring(i),
				label = util.basename(path),
				detail = path,
				icon = "file-pdf",
				actions = { click = "open=" .. path },
			}))
		end
		add(children, itemsNode(items, "open-folder"))
	end

	add(children, el("p", {
		class = "lr-hint",
		text = "在文件窗格里双击 PDF 会直接用文献阅读器打开；也可以运行命令面板的「文献阅读器：打开 PDF…」。",
	}))
	add(children, ui.node("row", { gap = "xs", wrap = true }, {
		ui.button("关闭", { size = "sm", variant = "quiet", actions = { click = "open-panel" } }),
	}))
	return ui.card({ ui.span("打开文献", "strong") }, children, { key = "open", icon = "folder" })
end

function M.recent(state)
	local children = {}
	local recent = state.recent
	if state.recentLoading and not recent then
		add(children, el("p", { class = "lr-hint", text = "正在读取最近文献…" }))
	elseif state.recentError then
		add(children, el("p", { class = "lr-error-text", text = state.recentError }))
	elseif recent and type(recent.documents) == "table" and #recent.documents > 0 then
		add(children, itemsNode(documentItems(recent.documents), "recent"))
	else
		add(children, el("p", { class = "lr-hint", text = "还没有最近阅读的文献。" }))
	end
	table.insert(children, ui.node("row", { gap = "xs", wrap = true }, {
		ui.button("刷新", { size = "sm", icon = "repeat", actions = { click = "reload" } }),
	}))

	local cards = {}
	if state.models and state.models.signed_in == false then
		table.insert(cards, ui.card({ ui.span("翻译需要 CodeBuddy 登录", "strong") }, {
			el("p", {
				class = "lr-hint",
				text = "登录后即可读取模型列表并开始翻译。命令面板也有「文献阅读器：登录 CodeBuddy」。",
			}),
			ui.button("登录 CodeBuddy", {
				key = "recent-login",
				size = "sm",
				icon = "log-in",
				actions = { click = "login" },
				disabled = state.loginJob and "正在登录" or false,
			}),
		}, { key = "recent-login", icon = "log-in" }))
	end
	table.insert(cards, ui.card({ ui.span("最近阅读", "strong") }, children, { key = "recent", icon = "newspaper" }))
	return ui.col({
		table.unpack(cards),
		el("p", {
			class = "lr-hint",
			text = "在文件窗格里对 PDF 选择「打开方式 → 文献阅读器」，或在命令面板运行「文献阅读器：打开 PDF…」。",
		}),
	})
end

function M.errorCard(state)
	local message = state.loadError or state.exeError or "未知错误"
	return ui.card({ ui.span("无法读取文献", "error strong") }, {
		el("p", { class = "lr-error-text", text = message }),
		el("p", {
			class = "lr-hint",
			text = "可点「刷新」重试；文件已移动或删除时，请从最近阅读或「打开 PDF…」重新选择。",
		}),
		el("p", { class = "lr-hint", text = "后端：" .. tostring(state.exe or "未就绪") }),
	}, { key = "error", tone = "error", icon = "warn" })
end

function M.missingCard(state)
	local children = {
		el("p", { class = "lr-error-text", text = tostring(state.missingPath or "") }),
		el("p", {
			class = "lr-hint",
			text = "文件可能已被移动或删除。从下面的最近阅读或同目录 PDF 重新选择。",
		}),
	}
	local folder = state.folderPdfs or {}
	if #folder > 0 then
		add(children, el("p", { class = "lr-hint", text = "同目录的 PDF" }))
		local items = {}
		for i, path in ipairs(folder) do
			table.insert(items, ui.node("item", {
				key = tostring(i),
				label = util.basename(path),
				detail = path,
				icon = "file-pdf",
				actions = { click = "open=" .. path },
			}))
		end
		add(children, itemsNode(items, "missing-folder"))
	end
	add(children, ui.button("知道了", {
		key = "dismiss-missing",
		size = "sm",
		actions = { click = "dismiss-missing" },
	}))
	return ui.card({ ui.span("上次的文献不存在", "warning strong") }, children, { key = "missing", tone = "warning", icon = "warn" })
end

function M.modelsCard(state)
	local children = {}
	local models = state.models
	if state.modelsError then
		add(children, el("p", { class = "lr-error-text", text = state.modelsError }))
		table.insert(children, ui.node("row", { gap = "xs", wrap = true }, {
			ui.button("重试", {
				size = "sm",
				icon = "repeat",
				actions = { click = "models-retry" },
				disabled = state.modelsLoading and "正在读取模型…" or false,
			}),
		}))
	elseif state.modelsLoading and not models then
		add(children, el("p", { class = "lr-hint", text = "正在读取模型…（最多约一分钟）" }))
	elseif models and type(models.models) == "table" and #models.models > 0 then
		local current = currentModelId(state)
		local items = {}
		for _, model in ipairs(models.models) do
			if type(model.id) == "string" then
				local detail = nil
				if type(model.multiplier) == "number" then
					detail = "×" .. tostring(model.multiplier)
				end
				table.insert(items, ui.node("item", {
					key = model.id,
					label = model.name or model.id,
					detail = detail,
					value = (model.id == current) and "当前" or nil,
					actions = { click = "model=" .. model.id },
				}))
			end
		end
		if #items > 0 then
			table.insert(children, ui.node("scroll", {
				key = "models-scroll",
				max = { h = "10lines" },
				bar = "auto",
			}, { ui.node("list", { key = "models" }, items) }))
		else
			add(children, el("p", { class = "lr-hint", text = "没有可用的模型。" }))
		end
	else
		local message = models and models.account_error
		add(children, el("p", { class = "lr-hint", text = message or "未登录，或没有可用的模型。" }))
	end
	if not state.modelsError and models and models.signed_in == false then
		table.insert(children, ui.node("row", { gap = "xs", wrap = true }, {
			ui.button("登录 CodeBuddy", {
				size = "sm",
				icon = "log-in",
				actions = { click = "login" },
				disabled = state.loginJob and "正在登录" or false,
			}),
		}))
	end
	table.insert(children, ui.node("row", { gap = "xs" }, {
		ui.button("取消", { size = "sm", variant = "quiet", actions = { click = "close-models" } }),
	}))

	return ui.card({ ui.span("选择翻译模型", "strong") }, children, { key = "models", icon = "sparkle" })
end

function M.loginCard(state)
	local login = state.login or {}
	local children = {}
	if login.url and login.url ~= "" then
		table.insert(children, ui.text({ ui.link(login.url, login.url) }))
	end
	add(children, el("p", {
		class = "lr-hint",
		text = login.url and "请在浏览器完成登录。" or "正在启动登录…",
	}))
	if login.state == "error" then
		add(children, el("p", { class = "lr-error-text", text = login.error or "登录失败" }))
	end
	if state.loginError then
		add(children, el("p", { class = "lr-error-text", text = state.loginError }))
	end
	local buttons = {
		ui.button("重新登录", {
			size = "sm",
			actions = { click = "login" },
			disabled = state.loginJob and "正在登录" or false,
		}),
	}
	if login.url and login.url ~= "" then
		table.insert(buttons, 1, ui.button("复制链接", {
			size = "sm",
			icon = "copy",
			actions = { click = "copy-url" },
		}))
	end
	table.insert(children, ui.node("row", { gap = "xs", wrap = true }, buttons))

	return ui.card({ ui.span(state.loginJob and "登录 CodeBuddy…" or "登录 CodeBuddy", "strong") }, children, {
		key = "login",
		icon = "log-in",
	})
end

function M.page(state)
	local snapshot = state.snapshot
	local pages = pageCount(state)
	local page = state.page
	local out = {}

	table.insert(out, ui.node("row", { gap = "xs", wrap = true, key = "page-head" }, {
		ui.text({ ui.span(("第 %d / %d 页"):format(page, pages), "muted") }),
		ui.button("上一页", {
			size = "sm",
			icon = "chev",
			keys = { "[" },
			actions = { click = "page=" .. tostring(page - 1) },
			disabled = page <= 1 and "已经是第一页" or false,
		}),
		ui.button("下一页", {
			size = "sm",
			icon = "chev-r",
			keys = { "]" },
			actions = { click = "page=" .. tostring(page + 1) },
			disabled = page >= pages and "已经是最后一页" or false,
		}),
	}))

	local blob = state.pageBlobs and state.pageBlobs[page]
	if blob then
		table.insert(out, ui.node("image", {
			key = "page-" .. tostring(page),
			blob = blob,
			alt = ("第 %d 页"):format(page),
			role = "page",
		}))
	elseif state.pageLoading == page then
		add(out, el("p", { class = "lr-hint", text = "正在生成页面图像…" }))
	else
		local errors = state.pageErrors
		add(out, el("p", {
			class = "lr-hint",
			text = errors and errors[page] and ("页面图像不可用：" .. errors[page]) or "页面图像不可用。",
		}))
	end

	return ui.col({ el("div", { class = "lr-page" }, out) })
end

local function statusSpans(state)
	local spans = {}
	local snapshot = state.snapshot
	if state.loading and not snapshot then
		table.insert(spans, ui.span("正在读取文献…", "muted"))
		return spans
	end
	local translation = snapshot and snapshot.translation or nil
	local done = translation and translation.done or (snapshot and snapshot.translated) or 0
	local total = translation and translation.total or (snapshot and snapshot.translatable) or 0
	table.insert(spans, ui.span(("已译 %d/%d"):format(done, total)))
	if translation and type(translation.rate) == "number" and translation.rate > 0 then
		table.insert(spans, ui.span((" · %s tok/s"):format(util.format1(translation.rate) or "0"), "num"))
	end
	local credits = snapshot and snapshot.credits
	if credits and type(credits.total) == "number" then
		table.insert(spans, ui.span((" · 积分 %s"):format(util.format1(credits.total) or "0"), "num"))
		if credits.complete == false then
			table.insert(spans, ui.span(" · 部分记录", "muted"))
		end
	end
	if state.modelsLoading then
		table.insert(spans, ui.span(" · 正在读取模型…", "muted"))
	end
	if state.layoutMessage then
		table.insert(spans, ui.span(" · " .. state.layoutMessage, "muted"))
	end
	local error = state.jobError
	if not error and translation and translation.state == "error" then
		error = translation.error or "翻译出错"
	end
	if error then
		table.insert(spans, ui.span(" · " .. error, "error"))
	end
	if state.loadError then
		table.insert(spans, ui.span(" · " .. state.loadError, "error"))
	end
	return spans
end

function M.dock(state)
	if not state.path or not state.exe then
		return nil
	end
	local snapshot = state.snapshot
	local running = state.job or (snapshot and snapshot.translation and snapshot.translation.state == "running")
	local signedIn = state.models and state.models.signed_in == true
	local row1 = {}
	table.insert(row1, ui.button("打开", {
		key = "open",
		role = "lr-open",
		size = "sm",
		icon = "folder",
		actions = { click = "open-panel" },
		on = state.showOpen,
		title = "打开文献（文件窗格双击 PDF 也可）",
	}))
	local name = modelName(state)
	table.insert(row1, ui.badge(name or "未选择模型", name and "accent" or "muted"))

	if running then
		table.insert(row1, ui.button(state.cancelling and "正在停止…" or "停止", {
			key = "stop",
			role = "lr-stop",
			size = "sm",
			variant = "danger",
			actions = { click = "stop" },
			disabled = state.cancelling and "正在停止翻译" or false,
		}))
	else
		local reason = false
		if state.loading and not snapshot then
			reason = "正在读取文献…"
		elseif state.modelsLoading then
			reason = "正在读取模型…"
		elseif state.modelsError then
			reason = state.modelsError
		elseif not signedIn then
			reason = "请先登录 CodeBuddy"
		end
		table.insert(row1, ui.button("翻译", {
			key = "translate",
			role = "lr-translate",
			size = "sm",
			variant = "primary",
			actions = { click = "translate" },
			disabled = reason,
		}))
	end

	table.insert(row1, ui.button("模型", {
		key = "models",
		role = "lr-models",
		size = "sm",
		actions = { click = "models" },
		on = state.showModels,
	}))
	if not signedIn and not state.modelsLoading then
		table.insert(row1, ui.button("登录", {
			key = "login",
			role = "lr-login",
			size = "sm",
			icon = "log-in",
			actions = { click = "login" },
			disabled = state.loginJob and "正在登录" or false,
		}))
	end
	table.insert(row1, ui.button(state.showZh and "译文" or "原文", {
		key = "zh",
		role = "lr-zh-toggle",
		size = "sm",
		actions = { click = "zh" },
		on = state.showZh,
		title = "显示或隐藏译文（快捷键 t）",
	}))
	table.insert(row1, ui.button(state.mode == "page" and "双语" or "原页", {
		key = "mode",
		role = "lr-mode",
		size = "sm",
		actions = { click = "mode" },
		on = state.mode == "page",
		title = "在原页与双语之间切换",
	}))
	table.insert(row1, ui.button("刷新", {
		key = "reload",
		role = "lr-reload",
		size = "sm",
		icon = "repeat",
		actions = { click = "reload" },
	}))

	local row2 = {}
	local status = ui.text(statusSpans(state))
	status.p = status.p or {}
	status.p.role = "lr-status"
	table.insert(row2, status)
	local translation = snapshot and snapshot.translation
	if running and translation and type(translation.total) == "number" and translation.total > 0 then
		local done = translation.done or 0
		table.insert(row2, ui.progress(util.clamp(done / translation.total, 0, 1), "翻译进度"))
	end

	return ui.col({
		ui.node("row", { gap = "xs", wrap = true, key = "actions" }, row1),
		ui.node("col", { gap = "sm", key = "status-row" }, row2),
	})
end

function M.backendCard(state)
	local backend = state.backend or {}
	local children = {}
	add(children, el("p", {
		class = "lr-hint",
		text = backend.message or "正在准备阅读内核…",
	}))
	if backend.state == "error" then
		add(children, el("p", { class = "lr-error-text", text = backend.error or "阅读内核准备失败。" }))
		add(children, el("p", {
			class = "lr-hint",
			text = "首次使用需要联网下载一次，之后离线可用。",
		}))
		add(children, ui.button("重试", {
			key = "backend-retry",
			size = "sm",
			icon = "repeat",
			actions = { click = "backend-retry" },
		}))
		if backend.url and backend.url ~= "" then
			add(children, el("p", { class = "lr-hint", text = backend.url }))
		end
	end
	return ui.card({ ui.span("阅读内核", "strong") }, children, { key = "backend", icon = "box" })
end

function M.view(state)
	local main = {}
	if state.showModels then
		table.insert(main, M.modelsCard(state))
	end
	if state.login and (state.login.state == "waiting" or state.login.state == "error") then
		table.insert(main, M.loginCard(state))
	end

	if state.missingPath then
		table.insert(main, M.missingCard(state))
	end
	if state.showOpen then
		table.insert(main, M.openCard(state))
	end
	local snapshot = state.snapshot
	if state.backend and state.backend.state ~= "ready" then
		table.insert(main, M.backendCard(state))
	elseif not state.path then
		table.insert(main, M.recent(state))
	elseif not snapshot then
		if state.loadError then
			table.insert(main, M.errorCard(state))
		else
			table.insert(main, el("p", { class = "lr-hint", text = "正在读取文献…" }))
		end
	elseif state.mode == "page" then
		table.insert(main, M.page(state))
	else
		table.insert(main, M.document(state))
	end

	local root = ui.col(main)
	root.p = root.p or {}
	root.p.role = "reader"
	return { main = root, dock = M.dock(state) }
end

return M
