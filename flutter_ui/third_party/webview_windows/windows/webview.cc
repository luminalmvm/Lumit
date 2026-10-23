#include "webview.h"

#include <shlwapi.h>
#include <wil/resource.h>
#include <wrl.h>

#include <algorithm>
#include <format>
#include <iostream>

#include "util/composition.desktop.interop.h"
#include "util/string_converter.h"
#include "webview_host.h"

using namespace Microsoft::WRL;

namespace {

inline void ConvertColor(COREWEBVIEW2_COLOR& webview_color, int32_t color) {
  webview_color.B = color & 0xFF;
  webview_color.G = (color >> 8) & 0xFF;
  webview_color.R = (color >> 16) & 0xFF;
  webview_color.A = (color >> 24) & 0xFF;
}

inline WebviewPermissionKind CW2PermissionKindToPermissionKind(
    COREWEBVIEW2_PERMISSION_KIND kind) {
  using k = COREWEBVIEW2_PERMISSION_KIND;
  switch (kind) {
    case k::COREWEBVIEW2_PERMISSION_KIND_MICROPHONE:
      return WebviewPermissionKind::Microphone;
    case k::COREWEBVIEW2_PERMISSION_KIND_CAMERA:
      return WebviewPermissionKind::Camera;
    case k::COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION:
      return WebviewPermissionKind::GeoLocation;
    case k::COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS:
      return WebviewPermissionKind::Notifications;
    case k::COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS:
      return WebviewPermissionKind::OtherSensors;
    case k::COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ:
      return WebviewPermissionKind::ClipboardRead;
    default:
      return WebviewPermissionKind::Unknown;
  }
}

inline COREWEBVIEW2_PERMISSION_STATE WebViewPermissionStateToCW2PermissionState(
    WebviewPermissionState state) {
  using s = COREWEBVIEW2_PERMISSION_STATE;
  switch (state) {
    case WebviewPermissionState::Allow:
      return s::COREWEBVIEW2_PERMISSION_STATE_ALLOW;
    case WebviewPermissionState::Deny:
      return s::COREWEBVIEW2_PERMISSION_STATE_DENY;
    default:
      return s::COREWEBVIEW2_PERMISSION_STATE_DEFAULT;
  }
}

}  // namespace

Webview::Webview(
    wil::com_ptr<ICoreWebView2CompositionController> composition_controller,
    WebviewHost* host, HWND hwnd, bool owns_window, bool offscreen_only)
    : composition_controller_(std::move(composition_controller)),
      host_(host),
      hwnd_(hwnd),
      owns_window_(owns_window) {
  webview_controller_ =
      composition_controller_.try_query<ICoreWebView2Controller3>();

  if (!webview_controller_ ||
      FAILED(webview_controller_->get_CoreWebView2(webview_.put()))) {
    return;
  }

  webview_controller_->put_BoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS);
  webview_controller_->put_ShouldDetectMonitorScaleChanges(FALSE);
  webview_controller_->put_RasterizationScale(1.0);

  wil::com_ptr<ICoreWebView2Settings> settings;
  if (SUCCEEDED(webview_->get_Settings(settings.put()))) {
    settings2_ = settings.try_query<ICoreWebView2Settings2>();

    settings->put_IsStatusBarEnabled(FALSE);
    settings->put_AreDefaultContextMenusEnabled(FALSE);
  }

  EnableSecurityUpdates();
  RegisterEventHandlers();

  is_valid_ = CreateSurface(host->compositor(), hwnd, offscreen_only);
}

Webview::~Webview() {
  if (owns_window_) {
    DestroyWindow(hwnd_);
  }
}

bool Webview::CreateSurface(
    winrt::com_ptr<ABI::Windows::UI::Composition::ICompositor> compositor,
    HWND hwnd, bool offscreen_only) {
  winrt::com_ptr<ABI::Windows::UI::Composition::IContainerVisual> root;
  if (FAILED(compositor->CreateContainerVisual(root.put()))) {
    return false;
  }

  surface_ = root.try_as<ABI::Windows::UI::Composition::IVisual>();
  assert(surface_);

  // initial size. doesn't matter as we resize the surface anyway.
  surface_->put_Size({1280, 720});
  surface_->put_IsVisible(true);

  // Create on-screen window for debugging purposes
  if (!offscreen_only) {
    window_target_ = util::TryCreateDesktopWindowTarget(compositor, hwnd);
    auto composition_target =
        window_target_
            .try_as<ABI::Windows::UI::Composition::ICompositionTarget>();
    if (composition_target) {
      composition_target->put_Root(surface_.get());
    }
  }

  winrt::com_ptr<ABI::Windows::UI::Composition::IVisual> webview_visual;
  compositor->CreateContainerVisual(
      reinterpret_cast<ABI::Windows::UI::Composition::IContainerVisual**>(
          webview_visual.put()));

  auto webview_visual2 =
      webview_visual.try_as<ABI::Windows::UI::Composition::IVisual2>();
  if (webview_visual2) {
    webview_visual2->put_RelativeSizeAdjustment({1.0f, 1.0f});
  }

  winrt::com_ptr<ABI::Windows::UI::Composition::IVisualCollection> children;
  root->get_Children(children.put());
  children->InsertAtTop(webview_visual.get());
  composition_controller_->put_RootVisualTarget(webview_visual2.get());

  webview_controller_->put_IsVisible(true);

  return true;
}

void Webview::EnableSecurityUpdates() {
  if (SUCCEEDED(webview_->CallDevToolsProtocolMethod(L"Security.enable", L"{}",
                                                     nullptr)) &&
      SUCCEEDED(webview_->GetDevToolsProtocolEventReceiver(
          L"Security.securityStateChanged",
          &devtools_protocol_event_receiver_))) {
    devtools_protocol_event_receiver_->add_DevToolsProtocolEventReceived(
        Callback<ICoreWebView2DevToolsProtocolEventReceivedEventHandler>(
            [this](ICoreWebView2* sender,
                   ICoreWebView2DevToolsProtocolEventReceivedEventArgs* args)
                -> HRESULT {
              if (devtools_protocol_event_callback_) {
                wil::unique_cotaskmem_string json_args;
                if (args->get_ParameterObjectAsJson(&json_args) == S_OK) {
                  std::string json = util::Utf8FromUtf16(json_args.get());
                  devtools_protocol_event_callback_(json.c_str());
                }
              }

              return S_OK;
            })
            .Get(),
        &event_registrations_.devtools_protocol_event_token_);
  }
}

void Webview::RegisterEventHandlers() {
  if (!webview_) {
    return;
  }

  webview_->add_ContentLoading(
      Callback<ICoreWebView2ContentLoadingEventHandler>(
          [this](ICoreWebView2* sender, IUnknown* args) -> HRESULT {
            if (loading_state_changed_callback_) {
              loading_state_changed_callback_(WebviewLoadingState::Loading);
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.content_loading_token_);

  webview_->add_NavigationCompleted(
      Callback<ICoreWebView2NavigationCompletedEventHandler>(
          [this](ICoreWebView2* sender,
                 ICoreWebView2NavigationCompletedEventArgs* args) -> HRESULT {
            BOOL is_success;
            args->get_IsSuccess(&is_success);
            if (!is_success && on_load_error_callback_) {
              COREWEBVIEW2_WEB_ERROR_STATUS web_error_status;
              args->get_WebErrorStatus(&web_error_status);
              on_load_error_callback_(web_error_status);
            }

            if (loading_state_changed_callback_) {
              loading_state_changed_callback_(
                  WebviewLoadingState::NavigationCompleted);
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.navigation_completed_token_);

  webview_->add_HistoryChanged(
      Callback<ICoreWebView2HistoryChangedEventHandler>(
          [this](ICoreWebView2* sender, IUnknown* args) -> HRESULT {
            if (history_changed_callback_) {
              BOOL can_go_back;
              BOOL can_go_forward;
              sender->get_CanGoBack(&can_go_back);
              sender->get_CanGoForward(&can_go_forward);
              history_changed_callback_({can_go_back, can_go_forward});
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.history_changed_token_);

  webview_->add_SourceChanged(
      Callback<ICoreWebView2SourceChangedEventHandler>(
          [this](ICoreWebView2* sender, IUnknown* args) -> HRESULT {
            LPWSTR wurl;
            if (url_changed_callback_ && webview_->get_Source(&wurl) == S_OK) {
              std::string url = util::Utf8FromUtf16(wurl);
              url_changed_callback_(url);
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.source_changed_token_);

  webview_->add_DocumentTitleChanged(
      Callback<ICoreWebView2DocumentTitleChangedEventHandler>(
          [this](ICoreWebView2* sender, IUnknown* args) -> HRESULT {
            LPWSTR wtitle;
            if (document_title_changed_callback_ &&
                webview_->get_DocumentTitle(&wtitle) == S_OK) {
              std::string title = util::Utf8FromUtf16(wtitle);
              document_title_changed_callback_(title);
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.document_title_changed_token_);

  composition_controller_->add_CursorChanged(
      Callback<ICoreWebView2CursorChangedEventHandler>(
          [this](ICoreWebView2CompositionController* sender,
                 IUnknown* args) -> HRESULT {
            HCURSOR cursor;
            if (cursor_changed_callback_ &&
                sender->get_Cursor(&cursor) == S_OK) {
              cursor_changed_callback_(cursor);
            }
            return S_OK;
          })
          .Get(),
      &event_registrations_.cursor_changed_token_);

  webview_controller_->add_GotFocus(
      Callback<ICoreWebView2FocusChangedEventHandler>(
          [this](ICoreWebView2Controller* sender, IUnknown* args) -> HRESULT {
            if (focus_changed_callback_) {
              focus_changed_callback_(true);
            }
            return S_OK;
          })
          .Get(),
      &event_registrations_.got_focus_token_);

  webview_controller_->add_LostFocus(
      Callback<ICoreWebView2FocusChangedEventHandler>(
          [this](ICoreWebView2Controller* sender, IUnknown* args) -> HRESULT {
            if (focus_changed_callback_) {
              focus_changed_callback_(false);
            }
            return S_OK;
          })
          .Get(),
      &event_registrations_.lost_focus_token_);

  webview_->add_WebMessageReceived(
      Callback<ICoreWebView2WebMessageReceivedEventHandler>(
          [this](ICoreWebView2* sender,
                 ICoreWebView2WebMessageReceivedEventArgs* args) -> HRESULT {
            wil::unique_cotaskmem_string wmessage;
            if (web_message_received_callback_ &&
                args->get_WebMessageAsJson(&wmessage) == S_OK) {
              const std::string message = util::Utf8FromUtf16(wmessage.get());
              web_message_received_callback_(message);
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.web_message_received_token_);

  webview_->add_PermissionRequested(
      Callback<ICoreWebView2PermissionRequestedEventHandler>(
          [this](ICoreWebView2* sender,
                 ICoreWebView2PermissionRequestedEventArgs* args) -> HRESULT {
            if (!permission_requested_callback_) {
              return S_OK;
            }

            wil::unique_cotaskmem_string wuri;
            COREWEBVIEW2_PERMISSION_KIND kind =
                COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
            BOOL is_user_initiated = false;

            if (args->get_Uri(&wuri) == S_OK &&
                args->get_PermissionKind(&kind) == S_OK &&
                args->get_IsUserInitiated(&is_user_initiated) == S_OK) {
              wil::com_ptr<ICoreWebView2Deferral> deferral;
              args->GetDeferral(deferral.put());

              const std::string uri = util::Utf8FromUtf16(wuri.get());
              permission_requested_callback_(
                  uri, CW2PermissionKindToPermissionKind(kind),
                  is_user_initiated == TRUE,
                  [deferral = std::move(deferral),
                   args = std::move(args)](WebviewPermissionState state) {
                    args->put_State(
                        WebViewPermissionStateToCW2PermissionState(state));
                    deferral->Complete();
                  });
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.permission_requested_token_);

  webview_->add_NewWindowRequested(
      Callback<ICoreWebView2NewWindowRequestedEventHandler>(
          [this](ICoreWebView2* sender,
                 ICoreWebView2NewWindowRequestedEventArgs* args) -> HRESULT {
            switch (popup_window_policy_) {
              case WebviewPopupWindowPolicy::Deny:
                args->put_Handled(TRUE);
                break;
              case WebviewPopupWindowPolicy::ShowInSameWindow:
                args->put_NewWindow(webview_.get());
                args->put_Handled(TRUE);
                break;
            }

            return S_OK;
          })
          .Get(),
      &event_registrations_.new_windows_requested_token_);

  webview_->add_ContainsFullScreenElementChanged(
      Callback<ICoreWebView2ContainsFullScreenElementChangedEventHandler>(
          [this](ICoreWebView2* sender, IUnknown* args) -> HRESULT {
            BOOL flag = FALSE;
            if (contains_fullscreen_element_changed_callback_ &&
                SUCCEEDED(sender->get_ContainsFullScreenElement(&flag))) {
              contains_fullscreen_element_changed_callback_(flag);
            }
            return S_OK;
          })
          .Get(),
      &event_registrations_.contains_fullscreen_element_changed_token_);
}

void Webview::SetSurfaceSize(size_t width, size_t height, float scale_factor) {
  if (!IsValid()) {
    return;
  }

  if (surface_ && width > 0 && height > 0) {
    scale_factor_ = scale_factor;
    auto scaled_width = width * scale_factor;
    auto scaled_height = height * scale_factor;

    RECT bounds;
    bounds.left = 0;
    bounds.top = 0;
    bounds.right = static_cast<LONG>(scaled_width);
    bounds.bottom = static_cast<LONG>(scaled_height);

    surface_->put_Size({scaled_width, scaled_height});
    webview_controller_->put_RasterizationScale(scale_factor);
    if (webview_controller_->put_Bounds(bounds) != S_OK) {
      std::cerr << "Setting webview bounds failed." << std::endl;
    }

    if (surface_size_changed_callback_) {
      surface_size_changed_callback_(width, height);
    }
  }
}

bool Webview::OpenDevTools() {
  if (!IsValid()) {
    return false;
  }
  webview_->OpenDevToolsWindow();
  return true;
}

bool Webview::ClearCookies() {
  if (!IsValid()) {
    return false;
  }
  return webview_->CallDevToolsProtocolMethod(L"Network.clearBrowserCookies",
                                              L"{}", nullptr) == S_OK;
}

bool Webview::ClearCache() {
  if (!IsValid()) {
    return false;
  }
  return webview_->CallDevToolsProtocolMethod(L"Network.clearBrowserCache",
                                              L"{}", nullptr) == S_OK;
}

bool Webview::SetCacheDisabled(bool disabled) {
  if (!IsValid()) {
    return false;
  }
  std::string json = std::format("{{\"disableCache\":{}}}", disabled);
  return webview_->CallDevToolsProtocolMethod(L"Network.setCacheDisabled",
                                              util::Utf16FromUtf8(json).c_str(),
                                              nullptr) == S_OK;
}

void Webview::SetPopupWindowPolicy(WebviewPopupWindowPolicy policy) {
  popup_window_policy_ = policy;
}

bool Webview::SetUserAgent(const std::string& user_agent) {
  if (settings2_) {
    return settings2_->put_UserAgent(util::Utf16FromUtf8(user_agent).c_str()) ==
           S_OK;
  }
  return false;
}

bool Webview::SetBackgroundColor(int32_t color) {
  if (!IsValid()) {
    return false;
  }

  COREWEBVIEW2_COLOR webview_color;
  ConvertColor(webview_color, color);

  // Semi-transparent backgrounds are not supported.
  // Valid alpha values are 0 or 255.
  if (webview_color.A > 0) {
    webview_color.A = 0xFF;
  }

  return webview_controller_->put_DefaultBackgroundColor(webview_color) == S_OK;
}

bool Webview::SetZoomFactor(double factor) {
  if (!IsValid()) {
    return false;
  }
  return webview_controller_->put_ZoomFactor(factor) == S_OK;
}

void Webview::SetCursorPos(double x, double y) {
  if (!IsValid()) {
    return;
  }

  POINT point;
  point.x = static_cast<LONG>(x * scale_factor_);
  point.y = static_cast<LONG>(y * scale_factor_);
  last_cursor_pos_ = point;

  // https://docs.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2?view=webview2-1.0.774.44
  composition_controller_->SendMouseInput(
      COREWEBVIEW2_MOUSE_EVENT_KIND::COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE,
      virtual_keys_.state(), 0, point);
}

void Webview::SetPointerUpdate(int32_t pointer,
                               WebviewPointerEventKind eventKind, double x,
                               double y, double size, double pressure) {
  if (!IsValid()) {
    return;
  }

  COREWEBVIEW2_POINTER_EVENT_KIND event =
      COREWEBVIEW2_POINTER_EVENT_KIND_UPDATE;
  UINT32 pointerFlags = POINTER_FLAG_NONE;
  switch (eventKind) {
    case WebviewPointerEventKind::Activate:
      event = COREWEBVIEW2_POINTER_EVENT_KIND_ACTIVATE;
      break;
    case WebviewPointerEventKind::Down:
      event = COREWEBVIEW2_POINTER_EVENT_KIND_DOWN;
      pointerFlags =
          POINTER_FLAG_DOWN | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
      break;
    case WebviewPointerEventKind::Enter:
      event = COREWEBVIEW2_POINTER_EVENT_KIND_ENTER;
      break;
    case WebviewPointerEventKind::Leave:
      event = COREWEBVIEW2_POINTER_EVENT_KIND_LEAVE;
      break;
    case WebviewPointerEventKind::Up:
      event = COREWEBVIEW2_POINTER_EVENT_KIND_UP;
      pointerFlags = POINTER_FLAG_UP;
      break;
    case WebviewPointerEventKind::Update:
      event = COREWEBVIEW2_POINTER_EVENT_KIND_UPDATE;
      pointerFlags =
          POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
      break;
  }

  POINT point;
  point.x = static_cast<LONG>(x * scale_factor_);
  point.y = static_cast<LONG>(y * scale_factor_);

  RECT rect;
  rect.left = point.x - 2;
  rect.right = point.x + 2;
  rect.top = point.y - 2;
  rect.bottom = point.y + 2;

  host_->CreateWebViewPointerInfo(
      [this, pointer, event, pointerFlags, point, rect, pressure](
          wil::com_ptr<ICoreWebView2PointerInfo> pointerInfo,
          std::unique_ptr<WebviewCreationError> error) {
        if (pointerInfo) {
          ICoreWebView2PointerInfo* pInfo = pointerInfo.get();
          pInfo->put_PointerId(pointer);
          pInfo->put_PointerKind(PT_TOUCH);
          pInfo->put_PointerFlags(pointerFlags);
          pInfo->put_TouchFlags(TOUCH_FLAG_NONE);
          pInfo->put_TouchMask(TOUCH_MASK_CONTACTAREA | TOUCH_MASK_PRESSURE);
          pInfo->put_TouchPressure(
              std::clamp((UINT32)(pressure == 0.0 ? 1024 : 1024 * pressure),
                         (UINT32)0, (UINT32)1024));
          pInfo->put_PixelLocationRaw(point);
          pInfo->put_TouchContactRaw(rect);
          composition_controller_->SendPointerInput(event, pInfo);
        }
      });
}

void Webview::SetPointerButtonState(WebviewPointerButton button, bool is_down) {
  if (!IsValid()) {
    return;
  }

  COREWEBVIEW2_MOUSE_EVENT_KIND kind;
  switch (button) {
    case WebviewPointerButton::Primary:
      virtual_keys_.set_isLeftButtonDown(is_down);
      kind = is_down ? COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN
                     : COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP;
      break;
    case WebviewPointerButton::Secondary:
      virtual_keys_.set_isRightButtonDown(is_down);
      kind = is_down ? COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN
                     : COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP;
      break;
    case WebviewPointerButton::Tertiary:
      virtual_keys_.set_isMiddleButtonDown(is_down);
      kind = is_down ? COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN
                     : COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP;
      break;
    default:
      kind = static_cast<COREWEBVIEW2_MOUSE_EVENT_KIND>(0);
  }

  composition_controller_->SendMouseInput(kind, virtual_keys_.state(), 0,
                                          last_cursor_pos_);
}

void Webview::SendScroll(double delta, bool horizontal) {
  // delta * 6 gives me a multiple of WHEEL_DELTA (120)
  constexpr auto kScrollMultiplier = 6;

  auto offset = static_cast<short>(delta * kScrollMultiplier);

  POINT point;
  point.x = 0;
  point.y = 0;

  if (horizontal) {
    composition_controller_->SendMouseInput(
        COREWEBVIEW2_MOUSE_EVENT_KIND_HORIZONTAL_WHEEL, virtual_keys_.state(),
        offset, point);
  } else {
    composition_controller_->SendMouseInput(COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL,
                                            virtual_keys_.state(), offset,
                                            point);
  }
}

void Webview::SetScrollDelta(double delta_x, double delta_y) {
  if (!IsValid()) {
    return;
  }

  if (delta_x != 0.0) {
    SendScroll(delta_x, true);
  }
  if (delta_y != 0.0) {
    SendScroll(delta_y, false);
  }
}

void Webview::LoadUrl(const std::string& url) {
  if (IsValid()) {
    webview_->Navigate(util::Utf16FromUtf8(url).c_str());
  }
}

void Webview::LoadStringContent(const std::string& content) {
  if (IsValid()) {
    webview_->NavigateToString(util::Utf16FromUtf8(content).c_str());
  }
}

bool Webview::Stop() {
  if (!IsValid()) {
    return false;
  }
  return SUCCEEDED(webview_->CallDevToolsProtocolMethod(L"Page.stopLoading",
                                                        L"{}", nullptr));
}

bool Webview::Reload() {
  if (!IsValid()) {
    return false;
  }
  return SUCCEEDED(webview_->Reload());
}

bool Webview::GoBack() {
  if (!IsValid()) {
    return false;
  }
  return SUCCEEDED(webview_->GoBack());
}

bool Webview::GoForward() {
  if (!IsValid()) {
    return false;
  }
  return SUCCEEDED(webview_->GoForward());
}

void Webview::AddScriptToExecuteOnDocumentCreated(
    const std::string& script,
    AddScriptToExecuteOnDocumentCreatedCallback callback) {
  if (IsValid()) {
    if (SUCCEEDED(webview_->AddScriptToExecuteOnDocumentCreated(
            util::Utf16FromUtf8(script).c_str(),
            Callback<
                ICoreWebView2AddScriptToExecuteOnDocumentCreatedCompletedHandler>(
                [callback](HRESULT result, LPCWSTR wsid) -> HRESULT {
                  std::string sid = util::Utf8FromUtf16(wsid);
                  callback(SUCCEEDED(result), sid);
                  return S_OK;
                })
                .Get()))) {
      return;
    }
  }

  callback(false, std::string());
}

void Webview::RemoveScriptToExecuteOnDocumentCreated(
    const std::string& script_id) {
  if (IsValid()) {
    webview_->RemoveScriptToExecuteOnDocumentCreated(
        util::Utf16FromUtf8(script_id).c_str());
  }
}

void Webview::ExecuteScript(const std::string& script,
                            ScriptExecutedCallback callback) {
  if (IsValid()) {
    if (SUCCEEDED(webview_->ExecuteScript(
            util::Utf16FromUtf8(script).c_str(),
            Callback<ICoreWebView2ExecuteScriptCompletedHandler>(
                [callback](HRESULT result, LPCWSTR json_result_object) {
                  callback(SUCCEEDED(result),
                           util::Utf8FromUtf16(json_result_object));
                  return S_OK;
                })
                .Get()))) {
      return;
    }
  }

  callback(false, std::string());
}

bool Webview::PostWebMessage(const std::string& json) {
  if (!IsValid()) {
    return false;
  }
  return webview_->PostWebMessageAsJson(util::Utf16FromUtf8(json).c_str()) ==
         S_OK;
}

bool Webview::Suspend() {
  if (!IsValid()) {
    return false;
  }

  wil::com_ptr<ICoreWebView2_3> webview;
  webview = webview_.query<ICoreWebView2_3>();
  if (!webview) {
    return false;
  }

  webview_controller_->put_IsVisible(false);
  return webview->TrySuspend(
             Callback<ICoreWebView2TrySuspendCompletedHandler>(
                 [](HRESULT error_code, BOOL is_successful) -> HRESULT {
                   return S_OK;
                 })
                 .Get()) == S_OK;
}

bool Webview::Resume() {
  if (!IsValid()) {
    return false;
  }

  wil::com_ptr<ICoreWebView2_3> webview;
  webview = webview_.query<ICoreWebView2_3>();
  if (!webview) {
    return false;
  }
  return webview->Resume() == S_OK &&
         webview_controller_->put_IsVisible(true) == S_OK;
}

bool Webview::SetVirtualHostNameMapping(
    const std::string& hostName, const std::string& path,
    WebviewHostResourceAccessKind accessKind) {
  if (!IsValid()) {
    return false;
  }

  wil::com_ptr<ICoreWebView2_3> webview;
  webview = webview_.query<ICoreWebView2_3>();
  if (!webview) {
    return false;
  }

  COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND accessKindIntValue =
      COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY;
  switch (accessKind) {
    case WebviewHostResourceAccessKind::Allow:
      accessKindIntValue = COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW;
      break;
    case WebviewHostResourceAccessKind::DenyCors:
      accessKindIntValue = COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS;
      break;
    case WebviewHostResourceAccessKind::Deny:
      accessKindIntValue = COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY;
      break;
  }

  return webview->SetVirtualHostNameToFolderMapping(
      util::Utf16FromUtf8(hostName).c_str(), util::Utf16FromUtf8(path).c_str(),
      accessKindIntValue);
}

bool Webview::ClearVirtualHostNameMapping(const std::string& hostName) {
  if (!IsValid()) {
    return false;
  }

  wil::com_ptr<ICoreWebView2_3> webview;
  webview = webview_.query<ICoreWebView2_3>();
  if (!webview) {
    return false;
  }

  return webview->ClearVirtualHostNameToFolderMapping(
      util::Utf16FromUtf8(hostName).c_str());
}

// Lumit: the request filter an extension's page is held to.

namespace {

// The parts of a URL the filter looks at. WebView2 hands over URLs already
// made canonical, with the scheme and host in lower case.
struct UrlParts {
  std::wstring scheme;
  std::wstring host;
  // Still percent-encoded, without the query or fragment.
  std::wstring path;
};

std::wstring Lower(std::wstring text) {
  for (auto& c : text) {
    if (c >= L'A' && c <= L'Z') {
      c += L'a' - L'A';
    }
  }
  return text;
}

UrlParts SplitUrl(const std::wstring& url) {
  UrlParts parts;
  const auto colon = url.find(L':');
  if (colon == std::wstring::npos) {
    return parts;
  }
  parts.scheme = Lower(url.substr(0, colon));
  if (url.compare(colon, 3, L"://") != 0) {
    return parts;
  }
  const auto start = colon + 3;
  const auto end = (std::min)(url.find_first_of(L"/?#", start), url.size());
  auto authority = url.substr(start, end - start);
  const auto at = authority.rfind(L'@');
  if (at != std::wstring::npos) {
    authority.erase(0, at + 1);
  }
  // An IPv6 address keeps its brackets, and a missing bracket leaves no host.
  parts.host = Lower(!authority.empty() && authority[0] == L'['
                         ? authority.substr(0, authority.find(L']') + 1)
                         : authority.substr(0, authority.find(L':')));
  if (end < url.size() && url[end] == L'/') {
    const auto stop = url.find_first_of(L"?#", end);
    parts.path = url.substr(
        end, stop == std::wstring::npos ? std::wstring::npos : stop - end);
  }
  return parts;
}

// The full path of an open file or folder, with every link followed.
std::wstring FinalPath(HANDLE handle) {
  std::wstring path(MAX_PATH, L'\0');
  auto size = GetFinalPathNameByHandleW(handle, path.data(),
                                        static_cast<DWORD>(path.size()),
                                        FILE_NAME_NORMALIZED);
  if (size >= path.size()) {
    path.resize(size);
    size = GetFinalPathNameByHandleW(handle, path.data(), size,
                                     FILE_NAME_NORMALIZED);
  }
  if (size == 0 || size >= path.size()) {
    return {};
  }
  path.resize(size);
  return path;
}

}  // namespace

bool Webview::LimitRequests(const std::string& own_host,
                            const std::string& folder,
                            const std::vector<std::string>& hosts,
                            const std::string& policy) {
  if (!IsValid() || environment_) {
    return false;
  }
  // An older runtime can't filter what a service or shared worker asks for,
  // so it gets no page at all.
  auto webview2 = webview_.try_query<ICoreWebView2_2>();
  auto webview22 = webview_.try_query<ICoreWebView2_22>();
  if (!webview2 || !webview22 ||
      FAILED(webview2->get_Environment(environment_.put()))) {
    return false;
  }
  wil::unique_hfile directory(CreateFileW(
      util::Utf16FromUtf8(folder).c_str(), 0,
      FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
      OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, nullptr));
  folder_ = directory ? FinalPath(directory.get()) : std::wstring();
  if (folder_.empty()) {
    return false;
  }
  own_host_ = Lower(util::Utf16FromUtf8(own_host));
  for (const auto& host : hosts) {
    hosts_.push_back(Lower(util::Utf16FromUtf8(host)));
  }
  policy_ = util::Utf16FromUtf8(policy);

  return SUCCEEDED(webview_->add_WebResourceRequested(
             Callback<ICoreWebView2WebResourceRequestedEventHandler>(
                 [this](ICoreWebView2* sender,
                        ICoreWebView2WebResourceRequestedEventArgs* args)
                     -> HRESULT {
                   OnRequest(args);
                   return S_OK;
                 })
                 .Get(),
             &event_registrations_.web_resource_requested_token_)) &&
         SUCCEEDED(
             webview22->AddWebResourceRequestedFilterWithRequestSourceKinds(
                 L"*", COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                 COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL));
}

void Webview::OnRequest(ICoreWebView2WebResourceRequestedEventArgs* args) {
  wil::com_ptr<ICoreWebView2WebResourceRequest> request;
  wil::unique_cotaskmem_string uri;
  if (FAILED(args->get_Request(request.put())) ||
      FAILED(request->get_Uri(&uri))) {
    return Answer(args, 403, nullptr, L"");
  }
  const auto url = SplitUrl(uri.get());
  if (url.host == own_host_) {
    return url.scheme == L"https" ? ServeFile(url.path, request.get(), args)
                                  : Answer(args, 403, nullptr, L"");
  }
  if (url.scheme == L"https" || url.scheme == L"wss") {
    for (const auto& host : hosts_) {
      if (url.host == host || url.host.ends_with(L"." + host)) {
        return;
      }
    }
  }
  // Nothing that goes out on the network.
  if (url.scheme == L"data" || url.scheme == L"blob" ||
      url.scheme == L"about") {
    return;
  }
  Answer(args, 403, nullptr, L"");
}

namespace {

// What a file is, by the end of its name, for the ones a page is made of.
const wchar_t* TypeOf(const std::wstring& path) {
  static const std::pair<const wchar_t*, const wchar_t*> kTypes[] = {
      {L".html", L"text/html"},        {L".htm", L"text/html"},
      {L".xhtml", L"application/xhtml+xml"},
      {L".css", L"text/css"},          {L".js", L"text/javascript"},
      {L".mjs", L"text/javascript"},   {L".json", L"application/json"},
      {L".map", L"application/json"},  {L".wasm", L"application/wasm"},
      {L".svg", L"image/svg+xml"},     {L".png", L"image/png"},
      {L".jpg", L"image/jpeg"},        {L".jpeg", L"image/jpeg"},
      {L".gif", L"image/gif"},         {L".webp", L"image/webp"},
      {L".avif", L"image/avif"},       {L".ico", L"image/x-icon"},
      {L".woff", L"font/woff"},        {L".woff2", L"font/woff2"},
      {L".ttf", L"font/ttf"},          {L".otf", L"font/otf"},
      {L".mp4", L"video/mp4"},         {L".webm", L"video/webm"},
      {L".mp3", L"audio/mpeg"},        {L".wav", L"audio/wav"},
      {L".ogg", L"audio/ogg"},         {L".txt", L"text/plain"},
      {L".xml", L"application/xml"},
  };
  const auto lower = Lower(path);
  for (const auto& [end, type] : kTypes) {
    if (lower.ends_with(end)) {
      return type;
    }
  }
  return L"application/octet-stream";
}

// A whole number of at most 18 digits, which can't overflow.
bool Number(const std::wstring& text, ULONGLONG* out) {
  if (text.empty() || text.size() > 18 ||
      text.find_first_not_of(L"0123456789") != std::wstring::npos) {
    return false;
  }
  *out = std::stoull(text);
  return true;
}

// The bytes a Range header asks for out of `size`, as `bytes=first-last`
// with either end left off, which is how sound and video are read.
bool Span(const std::wstring& header, ULONGLONG size, ULONGLONG* first,
          ULONGLONG* last) {
  const auto dash = header.find(L'-');
  if (size == 0 || header.rfind(L"bytes=", 0) != 0 ||
      dash == std::wstring::npos) {
    return false;
  }
  const auto from = header.substr(6, dash - 6);
  const auto to = header.substr(dash + 1);
  ULONGLONG a = 0;
  ULONGLONG b = 0;
  if (from.empty()) {
    // The last so many bytes.
    if (!Number(to, &b) || b == 0) {
      return false;
    }
    *first = b >= size ? 0 : size - b;
    *last = size - 1;
    return true;
  }
  if (!Number(from, &a) || a >= size || (!to.empty() && !Number(to, &b))) {
    return false;
  }
  *first = a;
  *last = to.empty() ? size - 1 : (std::min)(b, size - 1);
  return *last >= *first;
}

// The most of a file sent for a range that stops short of its end. A
// shorter answer than was asked for is allowed, and says so.
constexpr ULONGLONG kSpanLimit = 16 << 20;

}  // namespace

// A file of the extension's own, with the policy on it, and only for a page
// of its own or one being opened. A page from anywhere else can't load them.
void Webview::ServeFile(const std::wstring& path,
                        ICoreWebView2WebResourceRequest* request,
                        ICoreWebView2WebResourceRequestedEventArgs* args) {
  COREWEBVIEW2_WEB_RESOURCE_CONTEXT context;
  wil::unique_cotaskmem_string source;
  if (FAILED(args->get_ResourceContext(&context)) ||
      (context != COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT &&
       (FAILED(webview_->get_Source(&source)) ||
        SplitUrl(source.get()).host != own_host_))) {
    return Answer(args, 403, nullptr, L"");
  }
  std::wstring name = path;
  if (name.size() < 2 ||
      FAILED(UrlUnescapeW(name.data(), nullptr, nullptr,
                          URL_UNESCAPE_INPLACE | URL_UNESCAPE_AS_UTF8))) {
    return Answer(args, 404, nullptr, L"");
  }
  name.resize(wcslen(name.c_str()));
  // Plain names only, so it stays inside the folder before links are
  // followed too.
  std::wstring file = folder_;
  size_t start = 1;
  while (start <= name.size()) {
    const auto end = (std::min)(name.find(L'/', start), name.size());
    const auto part = name.substr(start, end - start);
    if (part.empty() || part == L"." || part == L".." ||
        part.find_first_of(L"\\:*?\"<>|") != std::wstring::npos ||
        std::any_of(part.begin(), part.end(),
                    [](wchar_t c) { return c < 0x20; })) {
      return Answer(args, 404, nullptr, L"");
    }
    file += L"\\" + part;
    start = end + 1;
  }
  wil::unique_hfile handle(CreateFileW(file.c_str(), GENERIC_READ,
                                       FILE_SHARE_READ, nullptr, OPEN_EXISTING,
                                       FILE_ATTRIBUTE_NORMAL, nullptr));
  const auto real = handle ? FinalPath(handle.get()) : std::wstring();
  const auto inside = folder_ + L"\\";
  BY_HANDLE_FILE_INFORMATION info;
  LARGE_INTEGER length;
  wil::com_ptr<IStream> body;
  if (real.size() <= inside.size() ||
      CompareStringOrdinal(real.c_str(), static_cast<int>(inside.size()),
                           inside.c_str(), static_cast<int>(inside.size()),
                           TRUE) != CSTR_EQUAL ||
      !GetFileInformationByHandle(handle.get(), &info) ||
      (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) ||
      !GetFileSizeEx(handle.get(), &length) ||
      FAILED(SHCreateStreamOnFileEx(real.c_str(),
                                    STGM_READ | STGM_SHARE_DENY_NONE, 0, FALSE,
                                    nullptr, body.put()))) {
    return Answer(args, 404, nullptr, L"");
  }
  const auto size = static_cast<ULONGLONG>(length.QuadPart);
  std::wstring headers = std::wstring(L"Content-Type: ") + TypeOf(real) +
                         L"\r\nContent-Security-Policy: " + policy_ +
                         L"\r\nCache-Control: no-cache"
                         L"\r\nAccept-Ranges: bytes";

  // Sound and video are read a piece at a time.
  wil::com_ptr<ICoreWebView2HttpRequestHeaders> asked;
  wil::unique_cotaskmem_string range;
  BOOL ranged = FALSE;
  ULONGLONG first = 0;
  ULONGLONG last = 0;
  if (SUCCEEDED(request->get_Headers(asked.put())) &&
      SUCCEEDED(asked->Contains(L"Range", &ranged)) && ranged &&
      SUCCEEDED(asked->GetHeader(L"Range", &range)) &&
      Span(range.get(), size, &first, &last)) {
    LARGE_INTEGER at;
    at.QuadPart = static_cast<LONGLONG>(first);
    if (FAILED(body->Seek(at, STREAM_SEEK_SET, nullptr))) {
      return Answer(args, 404, nullptr, L"");
    }
    // A range to the end reads the file to its end. Any other is read
    // here, since the stream would go on past it.
    if (last != size - 1) {
      last = (std::min)(last, first + kSpanLimit - 1);
      std::vector<BYTE> piece(static_cast<size_t>(last - first + 1));
      ULONG read = 0;
      if (FAILED(body->Read(piece.data(), static_cast<ULONG>(piece.size()),
                            &read)) ||
          read != piece.size()) {
        return Answer(args, 404, nullptr, L"");
      }
      body.attach(
          SHCreateMemStream(piece.data(), static_cast<UINT>(piece.size())));
      if (!body) {
        return Answer(args, 404, nullptr, L"");
      }
    }
    headers += std::format(
        L"\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}", first,
        last, size, last - first + 1);
    return Answer(args, 206, body.get(), headers);
  }
  headers += std::format(L"\r\nContent-Length: {}", size);
  Answer(args, 200, body.get(), headers);
}

void Webview::Answer(ICoreWebView2WebResourceRequestedEventArgs* args,
                     int status, IStream* body, const std::wstring& headers) {
  const wchar_t* reason = status == 200   ? L"OK"
                          : status == 206 ? L"Partial Content"
                          : status == 404 ? L"Not Found"
                                          : L"Forbidden";
  wil::com_ptr<ICoreWebView2WebResourceResponse> response;
  if (SUCCEEDED(environment_->CreateWebResourceResponse(
          body, status, reason, headers.c_str(), response.put()))) {
    args->put_Response(response.get());
  }
}
