#include "flutter_window.h"

#include <flutter/method_channel.h>
#include <flutter/standard_method_codec.h>
#include <shlobj.h>
#include <wrl/client.h>

#include <filesystem>
#include <optional>
#include <string>
#include <thread>
#include <variant>

#include "flutter/generated_plugin_registrant.h"
#include "resource.h"
#include "utils.h"

namespace {

// Put on the window while Lumit is kept running in the background, which is
// what a second start looks for (ForwardToBackgroundLumit).
constexpr wchar_t kKeepRunningProp[] = L"LumitKeepRunning";

// What the tray icon sends the window when it is clicked.
constexpr UINT kTrayMessage = WM_APP + 1;

// Marks a WM_COPYDATA as a second start's arguments.
constexpr ULONG_PTR kOpenAskedCopy = 0x4C554D54;

// This executable's full path.
std::wstring ExePath() {
  std::wstring path(32768, L'\0');
  path.resize(::GetModuleFileNameW(nullptr, path.data(),
                                   static_cast<DWORD>(path.size())));
  return path;
}

// Start Lumit in the tray when the person signs in, with a Run value of this
// executable and --background, or stop doing so.
void SetOpenAtSignIn(bool on) {
  constexpr wchar_t kRun[] = L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
  if (!on) {
    ::RegDeleteKeyValueW(HKEY_CURRENT_USER, kRun, L"Lumit");
    return;
  }
  const std::wstring line = L"\"" + ExePath() + L"\" --background";
  ::RegSetKeyValueW(HKEY_CURRENT_USER, kRun, L"Lumit", REG_SZ, line.c_str(),
                    static_cast<DWORD>((line.size() + 1) * sizeof(wchar_t)));
}

std::wstring Utf16FromUtf8(const std::string& text) {
  const int length = ::MultiByteToWideChar(
      CP_UTF8, 0, text.data(), static_cast<int>(text.size()), nullptr, 0);
  std::wstring wide(static_cast<size_t>(length > 0 ? length : 0), L'\0');
  ::MultiByteToWideChar(CP_UTF8, 0, text.data(), static_cast<int>(text.size()),
                        wide.data(), length);
  return wide;
}

// The window is invisible until Flutter's first frame arrives, which is what
// keeps a half-drawn editor off the screen. The cost of that is a start-up
// which never gets that far — an engine that cannot make a surface, an error
// before `runApp` — showing nothing whatever: Lumit sits in Task Manager with
// no window to close and nothing to report (reported against 0.3.0).
//
// So the first frame is given this long, and then the window is shown anyway.
// A window that has not painted yet can be looked at, moved and closed, and
// the Dart side puts its own failure screen in it when it has one. Ten seconds
// rather than one or two: a genuinely slow first start must not be interrupted
// by its own safety net.
constexpr UINT_PTR kShowAnywayTimer = 1;
constexpr UINT kShowAnywayAfterMs = 10000;

// The Start menu and a pinned taskbar button draw a shortcut's icon, not the
// window's. So every shortcut that starts this executable is given |icon| as
// its picture, or the executable's own when |icon| is empty.
//
// Explorer is then told each one changed, whether it did or not. It keeps the
// picture it drew first, and an update that swaps the executable's icon (a
// stable release for a nightly, or back) is otherwise not noticed for weeks.
void RefreshShortcutIcons(const std::wstring& icon) {
  wchar_t exe[MAX_PATH];
  if (::GetModuleFileNameW(nullptr, exe, MAX_PATH) == 0) {
    return;
  }
  // Up to and including the last backslash: the folder Lumit is installed in.
  const size_t home = std::wstring(exe).find_last_of(L'\\') + 1;
  // The Start menu's own folder, and the one that holds what is pinned to the
  // taskbar and to Start.
  for (const KNOWNFOLDERID& id : {FOLDERID_Programs, FOLDERID_UserPinned}) {
    wchar_t* folder = nullptr;
    if (FAILED(::SHGetKnownFolderPath(id, 0, nullptr, &folder))) {
      continue;
    }
    std::error_code error;
    for (std::filesystem::recursive_directory_iterator entry(folder, error),
         end;
         !error && entry != end; entry.increment(error)) {
      const std::wstring path = entry->path().wstring();
      if (_wcsicmp(entry->path().extension().c_str(), L".lnk") != 0) {
        continue;
      }
      Microsoft::WRL::ComPtr<IShellLinkW> link;
      Microsoft::WRL::ComPtr<IPersistFile> file;
      wchar_t target[MAX_PATH];
      if (FAILED(::CoCreateInstance(CLSID_ShellLink, nullptr,
                                    CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&link))) ||
          FAILED(link.As(&file)) ||
          FAILED(file->Load(path.c_str(), STGM_READWRITE)) ||
          FAILED(link->GetPath(target, MAX_PATH, nullptr, SLGP_RAWPATH)) ||
          _wcsicmp(target, exe) != 0) {
        continue;
      }
      // Only a picture of Lumit's own is replaced. One the user chose for the
      // shortcut is theirs to keep.
      wchar_t current[MAX_PATH];
      int index = 0;
      if (SUCCEEDED(link->GetIconLocation(current, MAX_PATH, &index)) &&
          (current[0] == L'\0' || _wcsnicmp(current, exe, home) == 0) &&
          _wcsicmp(current, icon.c_str()) != 0) {
        link->SetIconLocation(icon.c_str(), 0);
        file->Save(nullptr, TRUE);
      }
      ::SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, path.c_str(), nullptr);
    }
    ::CoTaskMemFree(folder);
  }
}

}  // namespace

bool ForwardToBackgroundLumit() {
  // A window carrying the mark, in a process started from this same file.
  // Another copy of Lumit elsewhere, a nightly beside a release for instance,
  // is left alone.
  struct Search {
    std::wstring exe;
    HWND found;
  } search{ExePath(), nullptr};
  ::EnumWindows(
      [](HWND hwnd, LPARAM lparam) -> BOOL {
        auto* search = reinterpret_cast<Search*>(lparam);
        if (!::GetPropW(hwnd, kKeepRunningProp)) {
          return TRUE;
        }
        DWORD pid = 0;
        ::GetWindowThreadProcessId(hwnd, &pid);
        HANDLE process =
            ::OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if (!process) {
          return TRUE;
        }
        std::wstring path(32768, L'\0');
        DWORD size = static_cast<DWORD>(path.size());
        const bool read =
            ::QueryFullProcessImageNameW(process, 0, path.data(), &size);
        ::CloseHandle(process);
        path.resize(read ? size : 0);
        if (_wcsicmp(path.c_str(), search->exe.c_str()) != 0) {
          return TRUE;
        }
        search->found = hwnd;
        return FALSE;
      },
      reinterpret_cast<LPARAM>(&search));
  if (!search.found) {
    return false;
  }
  // The arguments one after another, each ended by a nought.
  std::string asked;
  for (const std::string& argument : GetCommandLineArguments()) {
    asked += argument;
    asked += '\0';
  }
  COPYDATASTRUCT copy{kOpenAskedCopy, static_cast<DWORD>(asked.size()),
                      asked.data()};
  // Windows only lets the program just used put a window in front, which is
  // this one, so it hands that on.
  DWORD pid = 0;
  ::GetWindowThreadProcessId(search.found, &pid);
  ::AllowSetForegroundWindow(pid);
  DWORD_PTR took = 0;
  return ::SendMessageTimeoutW(search.found, WM_COPYDATA, 0,
                               reinterpret_cast<LPARAM>(&copy),
                               SMTO_ABORTIFHUNG, 5000, &took) != 0 &&
         took != 0;
}

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());

  // The zero-copy Viewer texture bridge: registers engine-created D3D
  // shared textures with Flutter over the 'lumit/viewer_texture' channel. Built
  // here, once the engine exists; a null registrar leaves the Viewer on the
  // read-back path (the Dart side falls back automatically).
  if (auto* registrar = flutter_controller_->engine()->GetRegistrarForPlugin(
          "LumitViewerTexture")) {
    viewer_texture_bridge_ = std::make_unique<ViewerTextureBridge>(registrar);
  }

  // A nightly may wear the regular icon (lib/state/app_icon.dart): a path
  // puts that .ico on the window, the taskbar and the Start menu, and none
  // puts the executable's own back. The window's lasts as long as Lumit runs.
  // The shortcuts keep theirs.
  flutter::MethodChannel<> app_icon(
      flutter_controller_->engine()->messenger(), "lumit/app_icon",
      &flutter::StandardMethodCodec::GetInstance());
  app_icon.SetMethodCallHandler(
      [this](const flutter::MethodCall<>& call,
             std::unique_ptr<flutter::MethodResult<>> result) {
        HICON icon = nullptr;
        std::wstring shortcut_icon;
        if (const auto* path = std::get_if<std::string>(call.arguments())) {
          // The length asked for with -1 counts the terminator, which the
          // string then holds as a spare character; c_str() stops before it.
          const int length = ::MultiByteToWideChar(CP_UTF8, 0, path->c_str(),
                                                   -1, nullptr, 0);
          std::wstring wide(static_cast<size_t>(length > 0 ? length : 0),
                            L'\0');
          ::MultiByteToWideChar(CP_UTF8, 0, path->c_str(), -1, wide.data(),
                                length);
          icon = static_cast<HICON>(
              ::LoadImageW(nullptr, wide.c_str(), IMAGE_ICON, 0, 0,
                           LR_LOADFROMFILE | LR_DEFAULTSIZE));
          if (icon) {
            shortcut_icon = wide.c_str();
          }
        }
        // On a thread of its own: reading every shortcut in the Start menu
        // takes about a seventh of a second, and this one draws the window.
        std::thread([shortcut_icon] {
          if (SUCCEEDED(::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) {
            RefreshShortcutIcons(shortcut_icon);
            ::CoUninitialize();
          }
        }).detach();
        // No path, or a file that would not load: the icon it was built with.
        if (!icon) {
          icon = ::LoadIcon(::GetModuleHandle(nullptr),
                            MAKEINTRESOURCE(IDI_APP_ICON));
        }
        ::SendMessage(GetHandle(), WM_SETICON, ICON_BIG,
                      reinterpret_cast<LPARAM>(icon));
        ::SendMessage(GetHandle(), WM_SETICON, ICON_SMALL,
                      reinterpret_cast<LPARAM>(icon));
        // The tray wears the window's icon.
        if (tray_shown_) {
          ShowTray(true);
        }
        result->Success();
      });

  // The window has no title bar (win32_window.cpp), so Flutter draws the
  // buttons one would hold and says here what was pressed
  // (lib/shell/window_frame.dart). Every call answers whether the window is
  // maximised, which is all the button's drawing needs to know.
  flutter::MethodChannel<> window(
      flutter_controller_->engine()->messenger(), "lumit/window",
      &flutter::StandardMethodCodec::GetInstance());
  window.SetMethodCallHandler(
      [this](const flutter::MethodCall<>& call,
             std::unique_ptr<flutter::MethodResult<>> result) {
        const HWND hwnd = GetHandle();
        const std::string& method = call.method_name();
        if (method == "drag") {
          // What a press on a title bar does. Flutter holds the mouse while a
          // button is down, and Windows will not move a window until it lets
          // go. Posted, so this call has answered before the move begins.
          ::ReleaseCapture();
          ::PostMessage(hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0);
        } else if (method == "minimise") {
          ::ShowWindow(hwnd, SW_MINIMIZE);
        } else if (method == "maximise") {
          ::ShowWindow(hwnd, ::IsZoomed(hwnd) ? SW_RESTORE : SW_MAXIMIZE);
        } else if (method == "close") {
          ::PostMessage(hwnd, WM_CLOSE, 0, 0);
        } else if (method == "background") {
          // Settings: whether closing the window keeps Lumit running in the
          // tray (lib/state/background.dart), and the tray menu's words.
          const auto* args = std::get_if<flutter::EncodableMap>(call.arguments());
          auto text = [args](const char* key) -> std::wstring {
            if (args) {
              auto found = args->find(flutter::EncodableValue(key));
              if (found != args->end()) {
                if (const auto* value = std::get_if<std::string>(&found->second)) {
                  return Utf16FromUtf8(*value);
                }
              }
            }
            return L"";
          };
          auto flag = [args](const char* key) {
            if (!args) {
              return false;
            }
            auto found = args->find(flutter::EncodableValue(key));
            const bool* value = found == args->end()
                                    ? nullptr
                                    : std::get_if<bool>(&found->second);
            return value && *value;
          };
          keep_running_ = flag("keep");
          tray_open_ = text("open");
          tray_quit_ = text("quit");
          if (keep_running_) {
            ::SetPropW(hwnd, kKeepRunningProp, reinterpret_cast<HANDLE>(1));
          } else {
            ::RemovePropW(hwnd, kKeepRunningProp);
          }
          ShowTray(keep_running_);
          SetOpenAtSignIn(keep_running_ && flag("signIn"));
          // Started at sign-in with nothing now to come back from: the window
          // is the only way back, so it shows.
          if (!keep_running_ && start_hidden_) {
            Reveal();
          }
        } else if (method == "show") {
          Reveal();
        }
        result->Success(flutter::EncodableValue(::IsZoomed(hwnd) != 0));
      });

  // What a second start passed on (WM_COPYDATA below), until Dart takes it,
  // as macOS's runner does with what the Finder asks for.
  flutter::MethodChannel<> open(flutter_controller_->engine()->messenger(),
                                "lumit/open",
                                &flutter::StandardMethodCodec::GetInstance());
  open.SetMethodCallHandler(
      [this](const flutter::MethodCall<>& call,
             std::unique_ptr<flutter::MethodResult<>> result) {
        if (call.method_name() != "take") {
          result->NotImplemented();
          return;
        }
        flutter::EncodableList asked;
        for (const std::string& argument : open_waiting_) {
          asked.emplace_back(argument);
        }
        open_waiting_.clear();
        result->Success(flutter::EncodableValue(asked));
      });

  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    if (!start_hidden_) {
      this->Show();
    }
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  ::SetTimer(GetHandle(), kShowAnywayTimer, kShowAnywayAfterMs, nullptr);

  return true;
}

void FlutterWindow::OnDestroy() {
  // Tear the texture bridge down first: it holds a registrar wrapper over the
  // engine, so it must go before the controller (and its engine) do.
  viewer_texture_bridge_ = nullptr;
  ShowTray(false);

  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Kept running in the background, closing the window only hides it. Asked
  // before Flutter, which would otherwise take the close for a quit. Quit in
  // the tray goes through Dart instead, and never comes here.
  if (message == WM_CLOSE && keep_running_) {
    ::ShowWindow(hwnd, SW_HIDE);
    Tell("lumit/window", "hidden");
    return 0;
  }

  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_TIMER:
      if (wparam == kShowAnywayTimer) {
        ::KillTimer(hwnd, kShowAnywayTimer);
        // Already up is the ordinary case: the first frame arrived and showed
        // it. Only a window nobody has seen is shown from here, so one the
        // user has since minimised is left where they put it.
        if (!::IsWindowVisible(hwnd) && !start_hidden_) {
          this->Show();
        }
        return 0;
      }
      break;
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
    case kTrayMessage:
      if (lparam == WM_LBUTTONUP) {
        Reveal();
      } else if (lparam == WM_RBUTTONUP) {
        TrayMenu();
      }
      return 0;
    case WM_COPYDATA: {
      // A second start's arguments (ForwardToBackgroundLumit).
      const auto* copy = reinterpret_cast<const COPYDATASTRUCT*>(lparam);
      if (copy->dwData != kOpenAskedCopy) {
        break;
      }
      const std::string all(static_cast<const char*>(copy->lpData),
                            copy->cbData);
      bool background = false;
      const size_t before = open_waiting_.size();
      for (size_t at = 0; at < all.size();) {
        size_t end = all.find('\0', at);
        if (end == std::string::npos) {
          end = all.size();
        }
        const std::string argument = all.substr(at, end - at);
        if (argument == "--background") {
          background = true;
        } else if (!argument.empty()) {
          open_waiting_.push_back(argument);
        }
        at = end + 1;
      }
      // Another start at sign-in has nothing to show.
      if (!background) {
        Reveal();
      }
      if (open_waiting_.size() > before) {
        Tell("lumit/open", "open");
      }
      return TRUE;
    }
  }

  // Explorer started again and the tray with it, empty.
  static const UINT taskbar_created =
      ::RegisterWindowMessageW(L"TaskbarCreated");
  if (message == taskbar_created && tray_shown_) {
    tray_shown_ = false;
    ShowTray(true);
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}

void FlutterWindow::Reveal() {
  const HWND hwnd = GetHandle();
  if (start_hidden_) {
    // The first showing, which knows whether the window was left maximised.
    start_hidden_ = false;
    Show();
  } else if (::IsIconic(hwnd)) {
    ::ShowWindow(hwnd, SW_RESTORE);
  } else {
    ::ShowWindow(hwnd, SW_SHOW);
  }
  ::SetForegroundWindow(hwnd);
  if (flutter_controller_) {
    flutter_controller_->ForceRedraw();
  }
}

void FlutterWindow::ShowTray(bool shown) {
  NOTIFYICONDATAW tray{};
  tray.cbSize = sizeof(tray);
  tray.hWnd = GetHandle();
  tray.uID = 1;
  if (!shown) {
    if (tray_shown_) {
      ::Shell_NotifyIconW(NIM_DELETE, &tray);
    }
    tray_shown_ = false;
    return;
  }
  tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
  tray.uCallbackMessage = kTrayMessage;
  tray.hIcon = reinterpret_cast<HICON>(
      ::SendMessage(GetHandle(), WM_GETICON, ICON_SMALL, 0));
  if (!tray.hIcon) {
    tray.hIcon = static_cast<HICON>(::LoadImageW(
        ::GetModuleHandle(nullptr), MAKEINTRESOURCE(IDI_APP_ICON), IMAGE_ICON,
        ::GetSystemMetrics(SM_CXSMICON), ::GetSystemMetrics(SM_CYSMICON), 0));
  }
  wcscpy_s(tray.szTip, L"Lumit");
  if (::Shell_NotifyIconW(tray_shown_ ? NIM_MODIFY : NIM_ADD, &tray)) {
    tray_shown_ = true;
  }
}

void FlutterWindow::TrayMenu() {
  const HWND hwnd = GetHandle();
  HMENU menu = ::CreatePopupMenu();
  ::AppendMenuW(menu, MF_STRING, 1, tray_open_.c_str());
  ::AppendMenuW(menu, MF_STRING, 2, tray_quit_.c_str());
  POINT at;
  ::GetCursorPos(&at);
  // Without this the menu stays up after a click somewhere else.
  ::SetForegroundWindow(hwnd);
  const UINT chosen = ::TrackPopupMenu(
      menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, at.x, at.y, 0,
      hwnd, nullptr);
  ::DestroyMenu(menu);
  if (chosen == 1) {
    Reveal();
  } else if (chosen == 2) {
    Tell("lumit/window", "quit");
  }
}

void FlutterWindow::Tell(const char* channel, const char* method) {
  if (!flutter_controller_) {
    return;
  }
  flutter::MethodChannel<>(flutter_controller_->engine()->messenger(), channel,
                           &flutter::StandardMethodCodec::GetInstance())
      .InvokeMethod(method, nullptr);
}
