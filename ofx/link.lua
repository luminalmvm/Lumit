-- Resolve-facing code runs inside Resolve. No external scripting or UIManager.
local root = ...
assert(root, 'Run the Lumit command from the Resolve Scripts menu.')
local function json(v)
    if type(v) == 'string' then
        return '"' .. v:gsub('[%z\1-\31\\"]', function(c)
            return string.format('\\u%04x', string.byte(c))
        end) .. '"'
    end
    if type(v) == 'boolean' or type(v) == 'number' then return tostring(v) end
    if type(v) ~= 'table' then return 'null' end
    local out = {}
    for k, value in (v[1] and ipairs or pairs)(v) do
        out[#out+1] = (type(k) == 'number' and '' or json(k)..':') .. json(value)
    end
    return (v[1] and '[' or '{') .. table.concat(out, ',') .. (v[1] and ']' or '}')
end

local ffi = ffi or require('ffi')
pcall(ffi.cdef, [[
int MultiByteToWideChar(unsigned int, unsigned long, const char*, int, wchar_t*, int);
int CreateDirectoryW(const wchar_t*, void*);
void* ShellExecuteW(void*, const wchar_t*, const wchar_t*, const wchar_t*, const wchar_t*, int);
int MessageBoxW(void*, const wchar_t*, const wchar_t*, unsigned int);
]])
local win = ffi.load('kernel32')
local shell = ffi.load('shell32')
local user = ffi.load('user32')
local function wide(s)
    local n = win.MultiByteToWideChar(65001, 0, s, #s, nil, 0)
    local w = ffi.new('wchar_t[?]', n+1)
    win.MultiByteToWideChar(65001, 0, s, #s, w, n)
    return w
end
local function fail(message) error(message, 0) end
local function frameRate(fps)
    for _, n in ipairs({24,30,60,120}) do
        if math.abs(fps-n*1000/1001)<0.0006 then return {num=n*1000,den=1001} end
    end
    return {num=math.floor(fps*1000+0.5),den=1000}
end
local binName = 'Lumit Link - DO NOT DELETE'
local canvasKey, projectKey = 'com.lumit.resolve.canvas', 'com.lumit.resolve.project'
local identityKey = 'com.lumit.resolve.projectId'
local function binding(pool)
    local folder, image, path, identity
    for _, candidate in ipairs(pool:GetRootFolder():GetSubFolderList() or {}) do
        if candidate:GetName() == binName then
            folder = folder or candidate
            for _, clip in ipairs(candidate:GetClipList() or {}) do
                local metadata = clip:GetThirdPartyMetadata() or {}
                if metadata[canvasKey] == '1' then
                    local target = metadata[projectKey]
                    local targetId = metadata[identityKey]
                    if target == '' then target = nil end
                    if targetId == '' then targetId = nil end
                    if (identity and targetId and identity ~= targetId) or
                        (not (identity and targetId) and path and target and
                         path:gsub('\\','/'):lower() ~= target:gsub('\\','/'):lower()) then
                        fail('The Lumit Link bin contains conflicting project bindings. Keep the intended canvas in this bin.')
                    end
                    if not image or (not identity and targetId) or (not path and target) then
                        folder, image, path, identity = candidate, clip, target, targetId
                    end
                end
            end
        end
    end
    return folder, image, path, identity
end
local function run()
    local app = resolve or bmd.scriptapp('Resolve')
    local project = app:GetProjectManager():GetCurrentProject()
    local timeline = project and project:GetCurrentTimeline()
    if not timeline then fail('Open a timeline and select clips first.') end
    local selected = timeline:GetSelectedClips()
    local videos, clips = {}, {}
    local first, last = math.huge, -math.huge
    local fps = tonumber(timeline:GetSetting('timelineFrameRate'))
    if not fps or fps <= 0 then fail('Cannot read the timeline frame rate.') end
    local rate=frameRate(fps); fps=rate.num/rate.den
    for _, clip in ipairs(selected or {}) do
        local track = clip:GetTrackTypeAndIndex()
        if track[1] == 'video' or track[1] == 'audio' then
            local media = clip:GetMediaPoolItem()
            local source = media and media:GetClipProperty() or {}
            local props = clip:GetProperties() or {}
            local speed = clip:GetSpeed() or {}
            local sourceFps=tonumber(source.FPS) or fps
            local sourceRate=frameRate(sourceFps > 0 and sourceFps or fps)
            local a, z = clip:GetStart(true), clip:GetEnd(true)
            local path = source['File Path'] or ''
            clips[#clips+1] = {name=clip:GetName(), kind=track[1], track=track[2],
                start=a, ['end']=z, path=path, enabled=clip:GetClipEnabled() and timeline:GetIsTrackEnabled(track[1],track[2]),
                source=track[1]=='audio' and clip:GetSourceStartTime() or
                    (clip:GetSourceStartFrame() or 0)*sourceRate.den/sourceRate.num,
                volume=props.AudioVolumeEnabled ~= false and (tonumber(props.AudioVolume) or 0) or 0,
                unsupported=path=='' and 'No source media (generator, title or nested clip)' or
                    (speed.Percentage and speed.Percentage ~= 100 and 'Retimed source' or nil)}
            if track[1]=='video' then
                videos[#videos+1]=clip
                first=math.min(first,a); last=math.max(last,z)
            end
        end
    end
    if #videos==0 then fail('Select at least one video clip. Selected audio is reference only.') end
    for _, c in ipairs(clips) do c.start=(c.start-first)/fps; c['end']=(c['end']-first)/fps end
    table.sort(clips, function(a,b)
        if a.kind ~= b.kind then return a.kind == 'audio' end
        if a.track ~= b.track then return a.track < b.track end
        return a.start < b.start
    end)
    local pool = project:GetMediaPool()
    local linkFolder, image, linkedProject, projectId = binding(pool)
    local previousFolder = pool:GetCurrentFolder()
    local directory = os.tmpname()
    os.remove(directory)
    if win.CreateDirectoryW(wide(directory), nil)==0 then fail('Cannot create a temporary setup folder.') end
    local codec = app:Fusion():MapPath('Fusion:'):gsub('[/\\]$',''):match('^(.*)[/\\]')..'/PGTools/libzstd.dll'
    local spec = {project=linkedProject, projectId=projectId, project_name=project:GetName(),
        name=timeline:GetName()..' · '..os.date('%H.%M.%S'),
        width=tonumber(timeline:GetSetting('timelineResolutionWidth')),
        height=tonumber(timeline:GetSetting('timelineResolutionHeight')),
        rate=rate, duration=(last-first)/fps,
        clips=clips, codec=codec}
    local file = assert(io.open(directory..'/selection.json','wb'))
    file:write(json(spec)); file:close()
    local seed, prepared, seedMedia, preparedMedia
    local canvas, canvasTrack, compound
    local disabled = {}
    local ok, problem = pcall(function()
        local name='Lumit setup '..directory:match('[^/\\]+$')
        seed=pool:CreateEmptyTimeline(name)
        if not seed then fail('Cannot prepare the Lumit effect.') end
        if not project:SetCurrentTimeline(seed) then fail('Resolve could not open the temporary setup timeline.') end
        local solid=seed:InsertGeneratorIntoTimeline('Solid Color')
        local carrier=seed:CreateCompoundClip({solid},{name=name..' source'})
        seedMedia=carrier:GetMediaPoolItem()
        if not carrier:AddEffect('ofx','Open FX/Lumit/Lumit Composition') then
            fail('Install Lumit Composition and restart Resolve first.')
        end
        if not seed:Export(directory..'/seed.drt',app.EXPORT_DRT,app.EXPORT_NONE) then
            fail('Cannot export the Lumit preset.')
        end
        if not project:SetCurrentTimeline(timeline) then fail('Resolve could not return to the selected timeline.') end
        pool:DeleteTimelines({seed}); seed=nil
        pool:DeleteClips({seedMedia}); seedMedia=nil
        local args='resolve-setup "'..directory..'"'
        local launched=shell.ShellExecuteW(nil,wide('open'),wide(root..'/lib/lumit-cli.exe'),wide(args),nil,0)
        if tonumber(ffi.cast('intptr_t',launched))<=32 then fail('Cannot start the Lumit setup utility.') end
        local ready=directory..'/ready.lua'
        local response
        for _=1,3600 do
            local f=io.open(ready,'rb')
            if f then f:close(); response=dofile(ready); break end
            bmd.wait(0.1)
        end
        if not response then fail('Lumit setup timed out; the timeline was left unchanged.') end
        if response.error then fail(response.error) end
        if response.cancelled then return end
        prepared=pool:ImportTimelineFromFile(directory..'/configured.drt')
        if not prepared then fail('Resolve rejected the configured effect.') end
        local template=prepared:GetItemListInTrack('video',1)[1]
        preparedMedia=template:GetMediaPoolItem()
        if not project:SetCurrentTimeline(timeline) then fail('Resolve could not return to the selected timeline.') end
        if not image then
            linkFolder=linkFolder or pool:AddSubFolder(pool:GetRootFolder(),binName)
            if not linkFolder or not pool:SetCurrentFolder(linkFolder) then fail('Cannot open the Lumit Link bin.') end
            local imported=pool:ImportMedia({root..'/Resolve/canvas.png'})
            pool:SetCurrentFolder(previousFolder)
            image=imported and imported[1]
            if not image then fail('Cannot import the transparent Lumit canvas.') end
            image:SetName('Lumit duration canvas - KEEP')
        end
        if not image:SetThirdPartyMetadata({[canvasKey]='1',[projectKey]=response.project,[identityKey]=response.projectId}) then
            fail('Cannot save the Lumit project binding in Resolve.')
        end
        if not timeline:AddTrack('video') then fail('Cannot add the Lumit canvas track.') end
        canvasTrack=timeline:GetTrackCount('video')
        local capacity=math.max(math.ceil(3600*fps),last-first)
        local rows=pool:AppendToTimeline({{mediaPoolItem=image,mediaType=1,
            startFrame=0,endFrame=capacity-1,recordFrame=first,trackIndex=canvasTrack}})
        canvas=rows and rows[1]
        if not canvas or not canvas:SetRightOffset(
            canvas:GetRightOffset()+canvas:GetDuration(true)-capacity,false,false) then
            fail('Cannot extend the transparent Lumit canvas.')
        end
        canvas:SetName('Lumit duration canvas - KEEP ENABLED')
        canvas:AddMarker(0,'Yellow','Leave this canvas in place',
            'Transparent one-hour canvas lets you extend the outer Lumit clip. '..
            'Original clips below are disabled; Lumit renders the visuals.',1)
        for _, clip in ipairs(videos) do
            if clip:GetClipEnabled() then
                if not clip:SetClipEnabled(false) then fail('Cannot disable the preserved source clip.') end
                disabled[#disabled+1]=clip
            end
        end
        local contents={}
        for _, clip in ipairs(videos) do contents[#contents+1]=clip end
        contents[#contents+1]=canvas
        compound=timeline:CreateCompoundClip(contents,{name='Lumit · '..response.name})
        if not compound or not template:CopyAttributes({compound},{videoPlugins=true}) then
            fail('Could not apply the linked effect. Original video remains inside the compound.')
        end
        if not compound:SetRightOffset(compound:GetRightOffset()+
            compound:GetDuration(true)-(last-first),false,false) then
            fail('Could not set the linked clip to the selected range.')
        end
        project:SetCurrentTimeline(timeline)
        app:GetProjectManager():SaveProject()
        print('Lumit linked: '..response.project)
    end)
    project:SetCurrentTimeline(timeline)
    pool:SetCurrentFolder(previousFolder)
    if not compound then
        for _, clip in ipairs(disabled) do clip:SetClipEnabled(true) end
        if canvas then timeline:DeleteClips({canvas},false) end
    end
    if canvasTrack and #timeline:GetItemListInTrack('video',canvasTrack)==0 then
        timeline:DeleteTrack('video',canvasTrack)
    end
    if seed then pool:DeleteTimelines({seed}) end
    if prepared then pool:DeleteTimelines({prepared}) end
    if seedMedia then pool:DeleteClips({seedMedia}) end
    if preparedMedia then pool:DeleteClips({preparedMedia}) end
    -- The one-shot helper has exited; only its known temporary files remain.
    for _, name in ipairs({'selection.json','selection.lum','request.json','imported.json',
        'seed.drt','configured.drt','ready.lua'}) do os.remove(directory..'/'..name) end
    os.remove(directory)
    if not ok then fail(problem) end
end
local ok, problem = pcall(run)
if not ok then
    print('Lumit: '..tostring(problem))
    user.MessageBoxW(nil,wide(tostring(problem)),wide('Create Lumit Composition'),0x10)
end
