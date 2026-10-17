//! UI Automation for the drawn flyout, so Narrator and tools like `winapp ui` see its buttons,
//! toggles, entry and history list the way they saw the WinUI controls.
//!
//! The window's root provider hands out element providers on demand. Each one is just an `Id`:
//! it looks itself up in the page's current element list on every call, so nothing goes stale as
//! the page changes. Calls arrive on the UI thread (`UseComThreading`), and a call that comes in
//! while the page is busy reports the element as unavailable rather than waiting.

// The UI Automation constants keep their Windows names, which match arms use as patterns.
#![allow(non_upper_case_globals)]

use std::rc::{Rc, Weak};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    IInvokeProvider, IInvokeProvider_Impl, IRawElementProviderFragment, IRawElementProviderFragment_Impl,
    IRawElementProviderFragmentRoot, IRawElementProviderFragmentRoot_Impl, IRawElementProviderSimple,
    IRawElementProviderSimple_Impl, IToggleProvider, IToggleProvider_Impl, IValueProvider, IValueProvider_Impl,
    NavigateDirection, NavigateDirection_FirstChild, NavigateDirection_LastChild, NavigateDirection_NextSibling,
    NavigateDirection_Parent, NavigateDirection_PreviousSibling, ProviderOptions, ProviderOptions_ServerSideProvider,
    ProviderOptions_UseComThreading, ToggleState, ToggleState_Off, ToggleState_On, UIA_AutomationFocusChangedEventId,
    UIA_AutomationIdPropertyId, UIA_ButtonControlTypeId, UIA_CONTROLTYPE_ID, UIA_ControlTypePropertyId,
    UIA_E_ELEMENTNOTAVAILABLE, UIA_E_ELEMENTNOTENABLED, UIA_EditControlTypeId, UIA_HasKeyboardFocusPropertyId,
    UIA_HelpTextPropertyId, UIA_InvokePatternId, UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId,
    UIA_ListControlTypeId, UIA_ListItemControlTypeId, UIA_LocalizedControlTypePropertyId, UIA_NamePropertyId,
    UIA_PATTERN_ID, UIA_PROPERTY_ID, UIA_TextControlTypeId, UIA_ToggleToggleStatePropertyId, UIA_TogglePatternId,
    UIA_ValuePatternId, UiaAppendRuntimeId, UiaClientsAreListening, UiaHostProviderFromHwnd, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaRect,
};
use windows::core::{BOOL, BSTR, Error, HRESULT, IUnknown, IUnknownImpl, Interface, PCWSTR, Result, implement};

use super::{Element, Id, Rect, Role};

/// What the providers need from the window.
pub trait Host {
    fn hwnd(&self) -> HWND;
    /// The elements on screen now, or `None` while the page is busy.
    fn elements(&self) -> Option<Vec<Element>>;
    /// The element with keyboard focus, when the window is active.
    fn focused(&self) -> Option<Id>;
    fn value(&self, id: Id) -> Option<String>;
    /// A rectangle in the window's DIPs as physical screen pixels.
    fn screen_rect(&self, rect: Rect) -> UiaRect;
    /// A physical screen point in the window's DIPs.
    fn client_point(&self, x: f64, y: f64) -> (f32, f32);
    fn invoke(&self, id: Id);
    fn focus(&self, id: Id);
    fn set_value(&self, id: Id, value: &str);
}

/// The provider for the window itself, returned from WM_GETOBJECT.
pub fn root(host: Weak<dyn Host>) -> IRawElementProviderSimple {
    RootProvider { host }.into()
}

/// Tells listening clients (Narrator) that keyboard focus moved to `id`.
pub fn raise_focus_changed(root: &IRawElementProviderSimple, host: Weak<dyn Host>, id: Id) {
    if !unsafe { UiaClientsAreListening() }.as_bool() {
        return;
    }
    if let Some(element) = element(root, host, id) {
        unsafe {
            let _ = UiaRaiseAutomationEvent(&element, UIA_AutomationFocusChangedEventId);
        }
    }
}

/// Tells listening clients that a toggle button or switch flipped.
pub fn raise_toggled(root: &IRawElementProviderSimple, host: Weak<dyn Host>, id: Id, on: bool) {
    if !unsafe { UiaClientsAreListening() }.as_bool() {
        return;
    }
    if let Some(element) = element(root, host, id) {
        let state = |on: bool| VARIANT::from(if on { ToggleState_On.0 } else { ToggleState_Off.0 });
        unsafe {
            let _ = UiaRaiseAutomationPropertyChangedEvent(&element, UIA_ToggleToggleStatePropertyId, &state(!on), &state(on));
        }
    }
}

fn element(root: &IRawElementProviderSimple, host: Weak<dyn Host>, id: Id) -> Option<IRawElementProviderSimple> {
    let root = root.cast::<IRawElementProviderFragmentRoot>().ok()?;
    Some(ElementProvider { host, root, id }.into())
}

fn options() -> ProviderOptions {
    ProviderOptions(ProviderOptions_ServerSideProvider.0 | ProviderOptions_UseComThreading.0)
}

fn unavailable() -> Error {
    Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}

/// S_OK with a null result: what UI Automation expects for "no such neighbor" or "no pattern".
fn none<T>() -> Result<T> {
    Err(Error::empty())
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IRawElementProviderFragmentRoot)]
struct RootProvider {
    host: Weak<dyn Host>,
}

impl RootProvider_Impl {
    fn host(&self) -> Result<Rc<dyn Host>> {
        self.host.upgrade().ok_or_else(unavailable)
    }

    fn provider(&self, id: Id) -> IRawElementProviderFragment {
        ElementProvider { host: self.host.clone(), root: self.to_interface(), id }.into()
    }

    /// The window's direct children: first or last.
    fn child(&self, last: bool) -> Result<IRawElementProviderFragment> {
        let elements = self.host()?.elements().ok_or_else(unavailable)?;
        let mut children = elements.iter().filter(|e| e.parent.is_none());
        let child = if last { children.next_back() } else { children.next() };
        child.map_or_else(none, |e| Ok(self.provider(e.id)))
    }
}

impl IRawElementProviderSimple_Impl for RootProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(options())
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        none()
    }

    fn GetPropertyValue(&self, _property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        // The window's own provider supplies its name, control type and bounds.
        Ok(VARIANT::default())
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        unsafe { UiaHostProviderFromHwnd(self.host()?.hwnd()) }
    }
}

impl IRawElementProviderFragment_Impl for RootProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        match direction {
            NavigateDirection_FirstChild => self.child(false),
            NavigateDirection_LastChild => self.child(true),
            _ => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // A root hosted in a window takes its runtime ID from the window.
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for RootProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let host = self.host()?;
        let (x, y) = host.client_point(x, y);
        let elements = host.elements().ok_or_else(unavailable)?;
        // Later elements draw on top, and list items come after their list.
        match elements.iter().rev().find(|e| e.rect.contains(x, y)) {
            Some(e) => Ok(self.provider(e.id)),
            None => none(),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        match self.host()?.focused() {
            Some(id) => Ok(self.provider(id)),
            None => none(),
        }
    }
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IInvokeProvider, IToggleProvider, IValueProvider)]
struct ElementProvider {
    host: Weak<dyn Host>,
    root: IRawElementProviderFragmentRoot,
    id: Id,
}

impl ElementProvider_Impl {
    fn lookup(&self) -> Result<(Rc<dyn Host>, Element, Vec<Element>)> {
        let host = self.host.upgrade().ok_or_else(unavailable)?;
        let elements = host.elements().ok_or_else(unavailable)?;
        let element = elements.iter().find(|e| e.id == self.id).cloned().ok_or_else(unavailable)?;
        Ok((host, element, elements))
    }

    fn provider(&self, id: Id) -> IRawElementProviderFragment {
        ElementProvider { host: self.host.clone(), root: self.root.clone(), id }.into()
    }

    fn sibling(&self, element: &Element, elements: &[Element], next: bool) -> Result<IRawElementProviderFragment> {
        let siblings: Vec<&Element> = elements.iter().filter(|e| e.parent == element.parent).collect();
        let index = siblings.iter().position(|e| e.id == self.id).ok_or_else(unavailable)?;
        let neighbor = if next { siblings.get(index + 1) } else { index.checked_sub(1).and_then(|i| siblings.get(i)) };
        neighbor.map_or_else(none, |e| Ok(self.provider(e.id)))
    }

    fn act(&self) -> Result<()> {
        let (host, element, _) = self.lookup()?;
        if !element.enabled {
            return Err(Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTENABLED as i32)));
        }
        host.invoke(self.id);
        Ok(())
    }
}

impl IRawElementProviderSimple_Impl for ElementProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(options())
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let (_, element, _) = self.lookup()?;
        let supported = match element.role {
            Role::Button | Role::ListItem => pattern == UIA_InvokePatternId,
            Role::ToggleButton { .. } | Role::Switch { .. } => pattern == UIA_TogglePatternId,
            Role::Edit => pattern == UIA_ValuePatternId,
            Role::Text | Role::List => false,
        };
        if supported { Ok(self.to_interface()) } else { none() }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let (host, element, _) = self.lookup()?;
        let text = |s: &str| VARIANT::from(BSTR::from(s));
        Ok(match property {
            UIA_ControlTypePropertyId => VARIANT::from(control_type(element.role).0),
            UIA_NamePropertyId => text(&element.name),
            UIA_HelpTextPropertyId => element.help.as_deref().map_or_else(VARIANT::default, text),
            UIA_AutomationIdPropertyId => element.automation_id.map_or_else(VARIANT::default, text),
            UIA_LocalizedControlTypePropertyId if matches!(element.role, Role::Switch { .. }) => text("toggle switch"),
            UIA_IsEnabledPropertyId => VARIANT::from(element.enabled),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(element.enabled && (element.focusable || element.role == Role::ListItem)),
            UIA_HasKeyboardFocusPropertyId => VARIANT::from(host.focused() == Some(self.id)),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl IRawElementProviderFragment_Impl for ElementProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let (_, element, elements) = self.lookup()?;
        match direction {
            NavigateDirection_Parent => match element.parent {
                Some(parent) => Ok(self.provider(parent)),
                None => self.root.cast(),
            },
            NavigateDirection_NextSibling => self.sibling(&element, &elements, true),
            NavigateDirection_PreviousSibling => self.sibling(&element, &elements, false),
            NavigateDirection_FirstChild | NavigateDirection_LastChild => {
                let mut children = elements.iter().filter(|e| e.parent == Some(self.id));
                let child = if direction == NavigateDirection_LastChild { children.next_back() } else { children.next() };
                child.map_or_else(none, |e| Ok(self.provider(e.id)))
            }
            _ => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        let ids = [UiaAppendRuntimeId as i32, self.id.number()];
        unsafe {
            let array = SafeArrayCreateVector(VT_I4, 0, ids.len() as u32);
            if array.is_null() {
                return Err(Error::from_thread());
            }
            for (index, id) in ids.iter().enumerate() {
                SafeArrayPutElement(array, &(index as i32), (id as *const i32).cast())?;
            }
            Ok(array)
        }
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let (host, element, _) = self.lookup()?;
        Ok(host.screen_rect(element.rect))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        let (host, _, _) = self.lookup()?;
        host.focus(self.id);
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.root.clone())
    }
}

impl IInvokeProvider_Impl for ElementProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        self.act()
    }
}

impl IToggleProvider_Impl for ElementProvider_Impl {
    fn Toggle(&self) -> Result<()> {
        self.act()
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        let (_, element, _) = self.lookup()?;
        Ok(match element.role {
            Role::ToggleButton { checked: true } | Role::Switch { on: true } => ToggleState_On,
            _ => ToggleState_Off,
        })
    }
}

impl IValueProvider_Impl for ElementProvider_Impl {
    fn SetValue(&self, value: &PCWSTR) -> Result<()> {
        let (host, _, _) = self.lookup()?;
        host.set_value(self.id, &unsafe { value.to_string() }.unwrap_or_default());
        Ok(())
    }

    fn Value(&self) -> Result<BSTR> {
        let (host, _, _) = self.lookup()?;
        Ok(BSTR::from(host.value(self.id).unwrap_or_default()))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(BOOL(0))
    }
}

fn control_type(role: Role) -> UIA_CONTROLTYPE_ID {
    match role {
        Role::Text => UIA_TextControlTypeId,
        // WinUI reports ToggleButton and ToggleSwitch as buttons with the Toggle pattern.
        Role::Button | Role::ToggleButton { .. } | Role::Switch { .. } => UIA_ButtonControlTypeId,
        Role::Edit => UIA_EditControlTypeId,
        Role::List => UIA_ListControlTypeId,
        Role::ListItem => UIA_ListItemControlTypeId,
    }
}
