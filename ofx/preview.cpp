// Frame delivery only. Lumit owns rendering, preview resolution and all caches.
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <shellapi.h>
#include <commdlg.h>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <string>
#include <vector>
#include <unordered_map>
#include <stdexcept>
#include <algorithm>
#include <cmath>
#include <charconv>
#include <emmintrin.h>
#include "offline_notice.h"
#ifdef LUMIT_CUDA
#include "delivery_gpu.h"
#endif
#ifdef LUMIT_OPENCL
#include "delivery_opencl.h"
#endif
#include "ofxImageEffect.h"
#include "ofxParam.h"
#include "ofxProperty.h"
#include "ofxMessage.h"

namespace {
OfxHost* host{};
OfxPropertySuiteV1* props{};
OfxImageEffectSuiteV1* effects{};
OfxParameterSuiteV1* params{};
OfxMessageSuiteV2* messages{};
struct TimelineSuite { OfxStatus (*getTime)(void*, double*); OfxStatus (*gotoTime)(void*, double); OfxStatus (*getTimeBounds)(void*, double*, double*); };
const TimelineSuite* timeline{};
void check(OfxStatus status) { if (status != kOfxStatOK) throw std::runtime_error("Resolve rejected an OFX operation."); }
OfxPropertySetHandle properties(OfxImageEffectHandle effect) { OfxPropertySetHandle p{}; check(effects->getPropertySet(effect, &p)); return p; }
OfxParamHandle parameter(OfxImageEffectHandle effect, const char* name) {
    OfxParamSetHandle set{}; OfxParamHandle p{};
    check(effects->getParamSet(effect, &set)); check(params->paramGetHandle(set, name, &p, nullptr)); return p;
}
std::string value(OfxImageEffectHandle effect, const char* name) { char* text{}; check(params->paramGetValue(parameter(effect, name), &text)); return text ? text : ""; }
std::wstring wide(const std::string& s) {
    int count = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s.data(), int(s.size()), nullptr, 0);
    if (!count) throw std::runtime_error("Invalid project name.");
    std::wstring w(count, 0); MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, s.data(), int(s.size()), w.data(), count); return w;
}
std::wstring normalized(std::wstring s) { for (auto& c : s) { if (c == L'/') c = L'\\'; c = towlower(c); } return s; }
struct Endpoint { unsigned port{}, pid{}; std::string token, path, id; };
struct Offline : std::runtime_error { using std::runtime_error::runtime_error; };
Endpoint endpoint(const std::string& project, const std::string& id = {}) {
    wchar_t appData[32768]{};
    if (!GetEnvironmentVariableW(L"LOCALAPPDATA", appData, 32768)) throw std::runtime_error("No local application folder.");
    const auto folder = std::filesystem::path(appData) / L"Lumit/Resolve Preview";
    const auto wanted = project.empty() ? std::wstring{} : normalized(wide(project));
    auto identity = id;
    // Path-only instances also follow a location the user has explicitly chosen.
    if (identity.empty() && !project.empty() && !std::filesystem::exists(wide(project)) && std::filesystem::exists(folder)) for (auto& file : std::filesystem::directory_iterator(folder)) {
        if (file.path().extension() != L".location") continue;
        std::ifstream input(file.path()); std::string path;
        while (std::getline(input, path)) if (!path.empty() && normalized(wide(path)) == wanted) {
            auto candidate = file.path().stem().string();
            if (!identity.empty() && identity != candidate) throw std::runtime_error("This path belongs to more than one linked Lumit project. Choose the current project path.");
            identity = candidate;
        }
    }
    Endpoint found{};
    if (std::filesystem::exists(folder)) for (auto& file : std::filesystem::directory_iterator(folder)) {
        if (file.path().extension() != L".txt") continue;
        std::ifstream input(file.path()); std::string port, token, pid, path, document;
        if (!std::getline(input, port) || !std::getline(input, token) || !std::getline(input, pid) || !std::getline(input, path)) continue;
        if (path.empty()) continue;
        std::getline(input, document);
        if (!identity.empty() && document != identity) continue;
        const auto actual = normalized(wide(path));
        if (identity.empty() && actual != wanted) continue;
        Endpoint candidate{0, 0, token, path, document};
        auto number = [](const std::string& text, unsigned& value) {
            auto result = std::from_chars(text.data(), text.data() + text.size(), value);
            return result.ec == std::errc{} && result.ptr == text.data() + text.size() && value;
        };
        if (!number(port, candidate.port) || candidate.port > 65535 || !number(pid, candidate.pid)) continue;
        HANDLE process = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, candidate.pid);
        if (!process) continue;
        FILETIME created{}, exited{}, kernel{}, user{}; WIN32_FILE_ATTRIBUTE_DATA fileData{};
        const bool alive = WaitForSingleObject(process, 0) == WAIT_TIMEOUT &&
            GetProcessTimes(process, &created, &exited, &kernel, &user) &&
            GetFileAttributesExW(file.path().c_str(), GetFileExInfoStandard, &fileData) &&
            CompareFileTime(&created, &fileData.ftLastWriteTime) <= 0;
        CloseHandle(process);
        if (!alive) continue;
        if (found.port) throw std::runtime_error("Two open Lumit projects match this link. Keep the intended project open in one window.");
        found = candidate;
    }
    if (!found.port) throw Offline("Open the linked project in Lumit, or use Open in Lumit to locate it.");
    return found;
}
void sendBytes(SOCKET s, const void* data, size_t count) {
    auto at = static_cast<const char*>(data);
    while (count) { int n = send(s, at, int(std::min<size_t>(count, 1 << 20)), 0); if (n <= 0) throw Offline("Lumit disconnected."); at += n; count -= n; }
}
void receive(SOCKET s, void* data, size_t count) {
    auto at = static_cast<char*>(data);
    while (count) { int n = recv(s, at, int(std::min<size_t>(count, 1 << 20)), 0); if (n <= 0) throw Offline("Lumit did not deliver the preview frame."); at += n; count -= n; }
}
void text(SOCKET s, const std::string& str) { uint32_t size = uint32_t(str.size()); sendBytes(s, &size, 4); sendBytes(s, str.data(), size); }
struct Delivery {
    std::mutex mutex;
    SOCKET socket{INVALID_SOCKET};
    std::string project;
    HANDLE mapping{};
    const unsigned char* pixels{};
    std::string surfaceName;
#ifdef LUMIT_CUDA
    CudaDelivery gpu;
#endif
#ifdef LUMIT_OPENCL
    OpenCLDelivery cl;
#endif
    ~Delivery() { disconnect(); }
    void disconnect(bool resetGpu = true) {
#ifdef LUMIT_CUDA
        if (resetGpu) gpu.reset();
#endif
#ifdef LUMIT_OPENCL
        if (resetGpu) cl.reset();
#endif
        if (pixels) UnmapViewOfFile(pixels); if (mapping) CloseHandle(mapping); pixels = nullptr; mapping = nullptr; surfaceName.clear(); if (socket != INVALID_SOCKET) closesocket(socket); socket = INVALID_SOCKET;
    }
    void connect(const std::string& name, const std::string& id = {}) {
        const auto key = id.empty() ? name : id;
        if (socket != INVALID_SOCKET && key == project) return;
        auto server = endpoint(name, id); disconnect();
        socket = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        if (socket == INVALID_SOCKET) throw std::runtime_error("Could not connect to Lumit.");
        DWORD timeout = 10000; BOOL yes = TRUE;
        setsockopt(socket, SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout));
        setsockopt(socket, SOL_SOCKET, SO_SNDTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout));
        setsockopt(socket, IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<char*>(&yes), sizeof(yes));
        sockaddr_in address{}; address.sin_family = AF_INET; address.sin_addr.s_addr = htonl(INADDR_LOOPBACK); address.sin_port = htons(u_short(server.port));
        if (::connect(socket, reinterpret_cast<sockaddr*>(&address), sizeof(address))) { disconnect(); throw Offline("The Lumit project is no longer open."); }
        text(socket, server.token); uint32_t version = 3; sendBytes(socket, &version, 4); project = key;
    }
};
// All instances use one bounded transfer buffer; this is not a frame cache.
Delivery delivery;
unsigned instances{}; // Protected by delivery.mutex, like the shared delivery resources.
struct Image {
    OfxPropertySetHandle handle{};
    ~Image() { if (handle) effects->clipReleaseImage(handle); }
};
// SSE2 is available on every Windows x64 CPU. Keep the same byte/255 result.
void copyRow(const unsigned char* source, float* output, int count) {
    const auto zero = _mm_setzero_si128();
    const auto divisor = _mm_set1_ps(255.0f);
    int at = 0;
    for (; at + 16 <= count; at += 16) {
        const auto bytes = _mm_loadu_si128(reinterpret_cast<const __m128i*>(source + at));
        const auto lo = _mm_unpacklo_epi8(bytes, zero), hi = _mm_unpackhi_epi8(bytes, zero);
        _mm_storeu_ps(output + at, _mm_div_ps(_mm_cvtepi32_ps(_mm_unpacklo_epi16(lo, zero)), divisor));
        _mm_storeu_ps(output + at + 4, _mm_div_ps(_mm_cvtepi32_ps(_mm_unpackhi_epi16(lo, zero)), divisor));
        _mm_storeu_ps(output + at + 8, _mm_div_ps(_mm_cvtepi32_ps(_mm_unpacklo_epi16(hi, zero)), divisor));
        _mm_storeu_ps(output + at + 12, _mm_div_ps(_mm_cvtepi32_ps(_mm_unpackhi_epi16(hi, zero)), divisor));
    }
    for (; at < count; ++at) output[at] = source[at] / 255.0f;
}
void scaleRow(const unsigned char* source, float* output, const std::vector<size_t>& offsets) {
    const auto zero = _mm_setzero_si128();
    const auto divisor = _mm_set1_ps(255.0f);
    for (auto offset : offsets) {
        int pixel; memcpy(&pixel, source + offset, sizeof(pixel));
        const auto bytes = _mm_unpacklo_epi8(_mm_cvtsi32_si128(pixel), zero);
        _mm_storeu_ps(output, _mm_div_ps(_mm_cvtepi32_ps(_mm_unpacklo_epi16(bytes, zero)), divisor));
        output += 4;
    }
}
void describe(OfxImageEffectHandle effect) {
    auto p = properties(effect);
    props->propSetString(p, kOfxPropLabel, 0, "Lumit Composition");
    props->propSetString(p, kOfxImageEffectPluginPropGrouping, 0, "Lumit");
    props->propSetString(p, kOfxImageEffectPropSupportedContexts, 0, kOfxImageEffectContextFilter);
    props->propSetString(p, kOfxImageEffectPropSupportedPixelDepths, 0, kOfxBitDepthFloat);
    props->propSetString(p, kOfxImageEffectPluginRenderThreadSafety, 0, kOfxImageEffectRenderInstanceSafe);
    props->propSetInt(p, kOfxImageEffectPropSupportsTiles, 0, 0);
    props->propSetInt(p, kOfxImageEffectPluginPropHostFrameThreading, 0, 0);
#ifdef LUMIT_CUDA
    if (CudaDelivery::supported()) {
        props->propSetString(p, "OfxImageEffectPropCudaRenderSupported", 0, "true");
        props->propSetString(p, "OfxImageEffectPropCudaStreamSupported", 0, "true");
    }
#endif
#ifdef LUMIT_OPENCL
    if (OpenCLDelivery::supported()) props->propSetString(p, "OfxImageEffectPropOpenCLRenderSupported", 0, "true");
#endif
}
void context(OfxImageEffectHandle effect) {
    OfxPropertySetHandle p{};
    for (auto name : {"Source", "Output"}) {
        check(effects->clipDefine(effect, name, &p));
        props->propSetString(p, kOfxImageEffectPropSupportedComponents, 0, kOfxImageComponentRGBA);
        props->propSetInt(p, kOfxImageEffectPropSupportsTiles, 0, 0);
    }
    OfxParamSetHandle set{}; check(effects->getParamSet(effect, &set));
    for (auto name : {"project", "composition", "pickProject", "choose", "open"}) {
        bool button = std::string(name) != "project" && std::string(name) != "composition";
        check(params->paramDefine(set, button ? kOfxParamTypePushButton : kOfxParamTypeString, name, &p));
        const char* label = std::string(name) == "project" ? "Project" : std::string(name) == "composition" ? "Composition" : std::string(name) == "pickProject" ? "Choose Project" : std::string(name) == "choose" ? "Choose Composition" : "Open in Lumit";
        props->propSetString(p, kOfxPropLabel, 0, label);
        if (!button) { props->propSetString(p, kOfxParamPropDefault, 0, ""); props->propSetInt(p, kOfxParamPropAnimates, 0, 0); props->propSetInt(p, kOfxParamPropSecret, 0, 1); }
        else if (std::string(name) != "pickProject") props->propSetInt(p, kOfxParamPropEnabled, 0, 0);
    }
    check(params->paramDefine(set, kOfxParamTypeString, "projectId", &p));
    props->propSetString(p, kOfxParamPropDefault, 0, "");
    props->propSetInt(p, kOfxParamPropSecret, 0, 1);
    props->propSetInt(p, kOfxParamPropAnimates, 0, 0);
    check(params->paramDefine(set, kOfxParamTypeBoolean, "matchDuration", &p));
    props->propSetString(p, kOfxPropLabel, 0, "Match duration to Resolve");
    props->propSetString(p, kOfxParamPropHint, 0, "This clip controls the composition end in Lumit when Resolve requests a frame. Enable on only one clip per composition.");
    props->propSetInt(p, kOfxParamPropDefault, 0, 0);
    props->propSetInt(p, kOfxParamPropAnimates, 0, 0);
    props->propSetInt(p, kOfxParamPropEnabled, 0, 0);
    check(params->paramDefine(set, kOfxParamTypePage, "controls", &p));
    int at = 0; for (auto name : {"pickProject", "choose", "open", "matchDuration"}) props->propSetString(p, kOfxParamPropPageChild, at++, name);
}
void updateControls(OfxImageEffectHandle effect) {
    const auto project = value(effect, "project"), composition = value(effect, "composition");
    for (auto name : {"pickProject", "choose", "open", "matchDuration"}) {
        OfxPropertySetHandle p{}; check(params->paramGetPropertySet(parameter(effect, name), &p));
        check(props->propSetInt(p, kOfxParamPropEnabled, 0, std::string(name) == "pickProject" || !project.empty()));
        if (std::string(name) == "pickProject") props->propSetString(p, kOfxParamPropHint, 0, project.empty() ? "Choose a Lumit project." : project.c_str());
        if (std::string(name) == "choose") props->propSetString(p, kOfxParamPropHint, 0, composition.empty() ? "Choose a composition from the open Lumit project." : composition.c_str());
    }
}
struct Composition { std::string id, name; };
struct Picker { std::vector<Composition> items; std::unordered_map<std::string, int> counts; std::string current; int selected{-1}; };
INT_PTR CALLBACK pickerProc(HWND window, UINT message, WPARAM wp, LPARAM lp) {
    try {
    auto picker = reinterpret_cast<Picker*>(GetWindowLongPtrW(window, DWLP_USER));
    if (message == WM_INITDIALOG) {
        picker = reinterpret_cast<Picker*>(lp); SetWindowLongPtrW(window, DWLP_USER, lp);
        int selected = 0;
        for (size_t i = 0; i < picker->items.size(); ++i) {
            auto& item = picker->items[i];
            auto label = item.name + (picker->counts[item.name] > 1 ? "  (" + item.id.substr(28) + ")" : "");
            auto row = SendDlgItemMessageW(window, 1001, LB_ADDSTRING, 0, reinterpret_cast<LPARAM>(wide(label).c_str()));
            SendDlgItemMessageW(window, 1001, LB_SETITEMDATA, row, i);
            if (item.id == picker->current || item.name == picker->current) selected = int(i);
        }
        for (size_t row = 0; row < picker->items.size(); ++row) {
            if (SendDlgItemMessageW(window, 1001, LB_GETITEMDATA, row, 0) == selected) SendDlgItemMessageW(window, 1001, LB_SETCURSEL, row, 0);
        }
        return TRUE;
    }
    if (message == WM_COMMAND && picker) {
        if (LOWORD(wp) == IDCANCEL) { EndDialog(window, 0); return TRUE; }
        if (LOWORD(wp) == IDOK || (LOWORD(wp) == 1001 && HIWORD(wp) == LBN_DBLCLK)) {
            auto selected = SendDlgItemMessageW(window, 1001, LB_GETCURSEL, 0, 0);
            if (selected != LB_ERR) { picker->selected = int(SendDlgItemMessageW(window, 1001, LB_GETITEMDATA, selected, 0)); EndDialog(window, 1); }
            return TRUE;
        }
    }
    return FALSE;
    } catch (...) { EndDialog(window, 0); return TRUE; }
}
void rememberProject(OfxImageEffectHandle effect, bool editedPath = false) {
    // Parameter writes belong to edit/create/sync actions, never Render.
    const auto project = value(effect, "project"), id = value(effect, "projectId");
    try {
        auto current = endpoint(project, editedPath ? "" : id);
        if (!current.id.empty() && current.id != id)
            check(params->paramSetValue(parameter(effect, "projectId"), current.id.c_str()));
        if (current.path != project)
            check(params->paramSetValue(parameter(effect, "project"), current.path.c_str()));
    } catch (...) {
        if (editedPath && !id.empty()) check(params->paramSetValue(parameter(effect, "projectId"), ""));
    }
}
void chooseComposition(OfxImageEffectHandle effect) {
    rememberProject(effect);
    // A short control connection, independent of any frame currently rendering.
    Delivery control; control.connect(value(effect, "project"), value(effect, "projectId"));
    uint32_t command = 1; sendBytes(control.socket, &command, 4);
    uint32_t header[6]{}; receive(control.socket, header, sizeof(header));
    if (header[0] != 0x33524d4c || header[1] != 2 || header[2] > 10000 || header[5] > 1u << 20) throw std::runtime_error("Cannot read the composition list. Update Lumit and OFX together.");
    std::string payload(header[5], 0); receive(control.socket, payload.data(), payload.size());
    Picker picker; picker.current = value(effect, "composition"); size_t at = 0;
    for (uint32_t i = 0; i < header[2]; ++i) {
        if (at + 40 > payload.size()) throw std::runtime_error("Invalid composition list.");
        auto id = payload.substr(at, 36); at += 36; uint32_t size{}; memcpy(&size, payload.data() + at, 4); at += 4;
        if (size > payload.size() - at) throw std::runtime_error("Invalid composition name.");
        auto name = payload.substr(at, size); at += size;
        ++picker.counts[name]; picker.items.push_back({id, name});
    }
    control.disconnect();
    if (picker.items.empty()) throw std::runtime_error("This Lumit project has no compositions.");
    HMODULE module{}; GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, reinterpret_cast<LPCWSTR>(&chooseComposition), &module);
    auto result = DialogBoxParamW(module, MAKEINTRESOURCEW(101), GetActiveWindow(), pickerProc, reinterpret_cast<LPARAM>(&picker));
    if (result == -1) throw std::runtime_error("Could not open the composition picker.");
    if (picker.selected >= 0) {
        auto& chosen = picker.items.at(picker.selected);
        const auto& selected = picker.counts[chosen.name] == 1 && !chosen.name.empty() ? chosen.name : chosen.id;
        // InstanceChanged is a permitted context for OFX parameter writes.
        check(params->paramSetValue(parameter(effect, "composition"), selected.c_str()));
    }
}
std::wstring quoted(const std::wstring& text) {
    std::wstring out=L"\""; size_t slashes=0;
    for(auto c:text) { if(c==L'\\'){++slashes;continue;} out.append(c==L'\"'?slashes*2+1:slashes,L'\\');slashes=0;out+=c; }
    out.append(slashes*2,L'\\');return out+L'\"';
}
void open(OfxImageEffectHandle effect) {
    rememberProject(effect);
    const auto project=value(effect,"project"), id=value(effect,"projectId");
    DWORD target=0;
    try {target=endpoint(project,id).pid;} catch(...) {}
    struct Search { DWORD pid; HWND window{}; unsigned count{}; } search{target};
    EnumWindows([](HWND w, LPARAM data)->BOOL { auto& s = *reinterpret_cast<Search*>(data); DWORD pid{}; GetWindowThreadProcessId(w, &pid); if ((!s.pid || pid == s.pid) && GetPropW(w, L"LumitOpen")) { s.window = w; ++s.count; } return TRUE; }, reinterpret_cast<LPARAM>(&search));
    const std::string control = "--resolve-open=" + value(effect, "composition");
    std::vector<std::string> arguments{control,"--resolve-path="+project};
    if(!id.empty()) arguments.push_back("--resolve-project="+id);
    if(search.count>1) throw std::runtime_error("Open this project in one Lumit window first.");
    if (!search.window) {
        HMODULE module{}; GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,reinterpret_cast<LPCWSTR>(&open),&module);
        wchar_t location[32768]{};GetModuleFileNameW(module,location,32768);
        auto folder=std::filesystem::path(location);for(int i=0;i<4;++i)folder=folder.parent_path();
        std::filesystem::path editor=L"lumit.exe";
        auto candidate = folder.parent_path()/L"lumit.exe";
        if(std::filesystem::is_regular_file(candidate)) editor=candidate;
        std::wstring args;
        for(auto& argument:arguments){if(!args.empty())args+=L" ";args+=quoted(wide(argument));}
        if(reinterpret_cast<INT_PTR>(ShellExecuteW(GetActiveWindow(),L"open",editor.c_str(),args.c_str(),nullptr,SW_SHOWNORMAL))<=32) throw std::runtime_error("Cannot start Lumit. Install Lumit or open this project first.");
        return;
    }
    std::string argument;
    for(auto& item:arguments){argument+=item;argument.push_back(0);}
    COPYDATASTRUCT data{0x4C554D54, DWORD(argument.size()), argument.data()}; DWORD_PTR result{};
    DWORD pid{};GetWindowThreadProcessId(search.window,&pid);AllowSetForegroundWindow(pid);
    if (!SendMessageTimeoutW(search.window, WM_COPYDATA, 0, reinterpret_cast<LPARAM>(&data), SMTO_ABORTIFHUNG, 2000, &result)) throw std::runtime_error("Lumit is busy.");
}
void pickProject(OfxImageEffectHandle effect) {
    wchar_t file[32768]{};
    const auto current = value(effect, "project");
    if (!current.empty()) {
        auto path = wide(current);
        if (path.size() < 32768) std::copy(path.begin(), path.end(), file);
    }
    OPENFILENAMEW dialog{}; dialog.lStructSize = sizeof(dialog); dialog.hwndOwner = GetActiveWindow();
    dialog.lpstrFilter = L"Lumit projects (*.lum)\0*.lum\0\0";
    dialog.lpstrFile = file; dialog.nMaxFile = 32768; dialog.lpstrTitle = L"Choose Lumit Project";
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if (!GetOpenFileNameW(&dialog)) {
        if (CommDlgExtendedError()) throw std::runtime_error("Could not open the project picker.");
        return;
    }
    const auto chosen = std::filesystem::path(file).u8string();
    if (current.empty() || normalized(wide(chosen)) != normalized(wide(current))) {
        check(params->paramSetValue(parameter(effect, "projectId"), ""));
        check(params->paramSetValue(parameter(effect, "composition"), ""));
        check(params->paramSetValue(parameter(effect, "matchDuration"), 0));
    }
    check(params->paramSetValue(parameter(effect, "project"), chosen.c_str()));
    updateControls(effect);
    open(effect);
}
void render(OfxImageEffectHandle effect, OfxPropertySetHandle args) {
    auto& self = delivery; std::lock_guard<std::mutex> lock(self.mutex);
    int cuda = 0; void* queue = nullptr;
    int opencl = 0; void* clQueue = nullptr;
#ifdef LUMIT_CUDA
    props->propGetInt(args, "OfxImageEffectPropCudaEnabled", 0, &cuda);
    if (cuda) props->propGetPointer(args, "OfxImageEffectPropCudaStream", 0, &queue);
#endif
#ifdef LUMIT_OPENCL
    props->propGetInt(args, "OfxImageEffectPropOpenCLEnabled", 0, &opencl);
    if (opencl) props->propGetPointer(args, "OfxImageEffectPropOpenCLCommandQueue", 0, &clQueue);
#endif
    double time{}, fps = 60; check(props->propGetDouble(args, kOfxPropTime, 0, &time));
    effects->getPropertySet(effect, &args); props->propGetDouble(args, kOfxImageEffectPropFrameRate, 0, &fps);
    if (!std::isfinite(fps) || fps <= 0) throw std::runtime_error("Invalid Resolve frame rate.");
    int match = 0; check(params->paramGetValue(parameter(effect, "matchDuration"), &match));
    double end = 0;
    if (match) {
        double first{}, last{};
        if (!timeline || timeline->getTimeBounds(effect, &first, &last) != kOfxStatOK ||
            !std::isfinite(first) || !std::isfinite(last) || last < first || last < 0)
            throw std::runtime_error("Resolve did not supply this clip's trimmed range.");
        end = (last + 1) / fps; // Resolve's end frame is inclusive; keep left trims in source time.
    }
    const auto project = value(effect, "project");
    try {
        unsigned width{}, height{}; uint64_t shared = 0;
        const unsigned char* pixels{}; bool offline = false;
        try {
        self.connect(project, value(effect, "projectId"));
        bool gpuRequest = cuda != 0;
#ifdef LUMIT_OPENCL
        if (opencl && !cuda) gpuRequest = self.cl.canShare(clQueue);
#endif
        uint32_t command = match ? (gpuRequest ? 5 : 4) : (gpuRequest ? 2 : 0); sendBytes(self.socket, &command, 4);
        text(self.socket, value(effect, "composition")); double seconds = time / fps; sendBytes(self.socket, &seconds, 8);
        if (match) sendBytes(self.socket, &end, 8);
        uint32_t header[6]{}; receive(self.socket, header, sizeof(header));
        if (header[0] != 0x33524d4c || header[4] > 256u << 20 || header[5] > 32768) throw std::runtime_error("Invalid Lumit preview response.");
        std::string payload(header[5], 0); receive(self.socket, payload.data(), payload.size());
        if (header[1] != 0 && header[1] != 3) throw std::runtime_error(payload);
        width = header[2]; height = header[3];
        if (!width || !height || uint64_t(width) * height * 4 != header[4]) throw std::runtime_error("Invalid preview dimensions.");
        if (header[1] == 3) {
            if (!gpuRequest || payload.size() != sizeof(shared)) throw std::runtime_error("Invalid GPU delivery surface.");
            memcpy(&shared, payload.data(), sizeof(shared));
            if (!shared) throw std::runtime_error("Empty GPU delivery surface.");
        } else if (payload.rfind("Local\\LumitPreview-", 0) != 0) throw std::runtime_error("Invalid CPU delivery surface.");
        if (!shared && self.surfaceName != payload) {
            if (self.pixels) UnmapViewOfFile(self.pixels);
            if (self.mapping) CloseHandle(self.mapping);
            self.pixels = nullptr; self.mapping = nullptr;
            self.mapping = OpenFileMappingW(FILE_MAP_READ, FALSE, wide(payload).c_str());
            if (!self.mapping) throw std::runtime_error("Cannot open the Lumit delivery surface.");
            self.pixels = static_cast<const unsigned char*>(MapViewOfFile(self.mapping, FILE_MAP_READ, 0, 0, header[4]));
            if (!self.pixels) throw std::runtime_error("Cannot map the Lumit delivery surface.");
            self.surfaceName = payload;
        }
        pixels = self.pixels;
        } catch (const Offline&) {
            self.disconnect(false);
            static const OfflineNotice notice;
            width = notice.width; height = notice.height; pixels = notice.pixels.data(); offline = true;
        }
        OfxImageClipHandle output{}; check(effects->clipGetHandle(effect, "Output", &output, nullptr));
        Image image; check(effects->clipGetImage(output, time, nullptr, &image.handle));
        void* bytes{}; int stride{}, bounds[4]{}, rod[4]{};
        check(props->propGetPointer(image.handle, kOfxImagePropData, 0, &bytes));
        check(props->propGetInt(image.handle, kOfxImagePropRowBytes, 0, &stride));
        check(props->propGetIntN(image.handle, kOfxImagePropBounds, 4, bounds));
        check(props->propGetIntN(image.handle, kOfxImagePropRegionOfDefinition, 4, rod));
        if (!bytes || rod[2] <= rod[0] || rod[3] <= rod[1]) throw std::runtime_error("Invalid Resolve output image.");
        if (bounds[2] <= bounds[0] || bounds[3] <= bounds[1] || effects->abort(effect)) return;
#ifdef LUMIT_CUDA
        if (cuda) {
            if (!shared && !offline && (width != 1 || height != 1 || pixels[3])) throw std::runtime_error("Lumit did not deliver a GPU frame.");
            self.gpu.copy(shared, width, height, bytes, bounds, rod, stride, queue, offline ? pixels : nullptr);
            return;
        }
#endif
#ifdef LUMIT_OPENCL
        if (opencl) {
            self.cl.copy(shared, width, height, pixels, bytes, bounds, rod, stride, clQueue);
            return;
        }
#endif
        const bool direct = width == unsigned(rod[2] - rod[0]) && height == unsigned(rod[3] - rod[1]) &&
            bounds[0] >= rod[0] && bounds[2] <= rod[2] && bounds[1] >= rod[1] && bounds[3] <= rod[3];
        std::vector<size_t> offsets;
        if (!direct) for (int x = bounds[0]; x < bounds[2]; ++x) {
            offsets.push_back(size_t(std::clamp(int(((2LL * (x - rod[0]) + 1) * width) / (2LL * (rod[2] - rod[0]))), 0, int(width) - 1)) * 4);
        }
        int previousY = -1; float* previousRow = nullptr;
        for (int y = bounds[1]; y < bounds[3]; ++y) {
            if (effects->abort(effect)) return;
            auto row = reinterpret_cast<float*>(static_cast<char*>(bytes) + ptrdiff_t(y - bounds[1]) * stride);
            if (direct) {
                auto pixel = pixels + (size_t(rod[3] - 1 - y) * width + unsigned(bounds[0] - rod[0])) * 4;
                copyRow(pixel, row, (bounds[2] - bounds[0]) * 4);
                continue;
            }
            const int sy = std::clamp(int(((2LL * (rod[3] - y) - 1) * height) / (2LL * (rod[3] - rod[1]))), 0, int(height) - 1);
            if (sy == previousY) memcpy(row, previousRow, offsets.size() * 4 * sizeof(float));
            else scaleRow(pixels + size_t(sy) * width * 4, row, offsets);
            previousY = sy; previousRow = row;
        }
    } catch (...) { self.disconnect(); throw; }
}
OfxStatus entry(const char* action, const void* handle, OfxPropertySetHandle in, OfxPropertySetHandle out) {
    auto effect = (OfxImageEffectHandle)handle;
    try {
        std::string a = action;
        if (a == kOfxActionLoad) {
            props = (OfxPropertySuiteV1*)host->fetchSuite(host->host, kOfxPropertySuite, 1);
            effects = (OfxImageEffectSuiteV1*)host->fetchSuite(host->host, kOfxImageEffectSuite, 1);
            params = (OfxParameterSuiteV1*)host->fetchSuite(host->host, kOfxParameterSuite, 1);
            messages = (OfxMessageSuiteV2*)host->fetchSuite(host->host, kOfxMessageSuite, 2);
            timeline = static_cast<const TimelineSuite*>(host->fetchSuite(host->host, "OfxTimeLineSuite", 1));
            WSADATA data{}; if (!props || !effects || !params || WSAStartup(MAKEWORD(2, 2), &data)) return kOfxStatErrMissingHostFeature;
        } else if (a == kOfxActionUnload) {
            std::lock_guard<std::mutex> lock(delivery.mutex);
            delivery.disconnect(); WSACleanup();
        } else if (a == kOfxActionDescribe) describe(effect);
        else if (a == kOfxImageEffectActionDescribeInContext) context(effect);
        else if (a == kOfxActionCreateInstance || a == kOfxActionSyncPrivateData) {
            rememberProject(effect); updateControls(effect);
            if (a == kOfxActionCreateInstance) { std::lock_guard<std::mutex> lock(delivery.mutex); ++instances; }
        }
        else if (a == kOfxActionDestroyInstance) {
            std::lock_guard<std::mutex> lock(delivery.mutex);
            if (instances && --instances == 0) delivery.disconnect();
        }
        else if (a == kOfxImageEffectActionRender) render(effect, in);
        else if (a == kOfxActionInstanceChanged) {
            char* name{}; check(props->propGetString(in, kOfxPropName, 0, &name));
            if (name && std::string(name) == "project") rememberProject(effect, true);
            if (name && std::string(name) == "pickProject") pickProject(effect);
            if (name && std::string(name) == "open") open(effect);
            if (name && std::string(name) == "choose") chooseComposition(effect);
            updateControls(effect);
        } else if (a == kOfxImageEffectActionGetRegionsOfInterest) {
            double empty[4]{}; check(props->propSetDoubleN(out, "OfxImageClipPropRoI_Source", 4, empty));
        } else if (a == kOfxImageEffectActionGetClipPreferences) {
            check(props->propSetInt(out, kOfxImageEffectFrameVarying, 0, 1));
            check(props->propSetString(out, kOfxImageEffectPropPreMultiplication, 0, kOfxImagePreMultiplied));
        } else if (a == kOfxImageEffectActionGetRegionOfDefinition) {
            // This picture is independent of the preserved source, including gaps.
            double size[2]{}, offset[2]{}; auto p = properties(effect);
            if (props->propGetDoubleN(p, kOfxImageEffectPropProjectSize, 2, size) != kOfxStatOK) return kOfxStatReplyDefault;
            props->propGetDoubleN(p, kOfxImageEffectPropProjectOffset, 2, offset);
            double rect[4]{offset[0], offset[1], offset[0] + size[0], offset[1] + size[1]};
            check(props->propSetDoubleN(out, kOfxImageEffectPropRegionOfDefinition, 4, rect));
        }
        else return kOfxStatReplyDefault;
        if (messages && effect) messages->clearPersistentMessage(effect);
        return kOfxStatOK;
    } catch (const std::exception& e) { if (messages && effect) messages->setPersistentMessage(effect, kOfxMessageError, "lumit-preview", "%s", e.what()); return kOfxStatFailed; }
    catch (...) { return kOfxStatFailed; }
}
void setHost(OfxHost* h) { host = h; }
OfxPlugin plugin{kOfxImageEffectPluginApi, 1, "com.lumitlab.resolve.preview", 1, 0, setHost, entry};
}
extern "C" __declspec(dllexport) int OfxGetNumberOfPlugins() { return 1; }
extern "C" __declspec(dllexport) OfxPlugin* OfxGetPlugin(int index) { return index == 0 ? &plugin : nullptr; }
