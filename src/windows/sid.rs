// Take a look at the license at the top of the repository in the LICENSE file.

use std::{fmt::Display, str::FromStr};

use windows::core::{PCWSTR, PWSTR};
#[cfg(feature = "user")]
use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows::Win32::Security::{CopySid, GetLengthSid, IsValidSid, PSID};
#[cfg(feature = "user")]
use windows::Win32::Security::{LookupAccountSidW, SidTypeUnknown};

use crate::sys::utils::to_utf8_str;

#[doc = include_str!("../../md_doc/sid.md")]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sid {
    sid: Vec<u8>,
}

impl Sid {
    /// Creates an `Sid` by making a copy of the given raw SID.
    pub(crate) unsafe fn from_psid(psid: PSID) -> Option<Self> {
        if psid.is_invalid() {
            return None;
        }

        if !IsValidSid(psid).as_bool() {
            return None;
        }

        let length = GetLengthSid(psid);

        let mut sid = vec![0; length as usize];

        if CopySid(length, PSID(sid.as_mut_ptr().cast()), psid).is_err() {
            sysinfo_debug!("CopySid failed: {:?}", std::io::Error::last_os_error());
            return None;
        }

        // We are making assumptions about the SID internal structure,
        // and these only hold if the revision is 1
        // https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-sid
        // Namely:
        // 1. SIDs can be compared directly (memcmp).
        // 2. Following from this, to hash a SID we can just hash its bytes.
        // These are the basis for deriving PartialEq, Eq, and Hash.
        // And since we also need PartialOrd and Ord, we might as well derive them
        // too. The default implementation will be consistent with Eq,
        // and we don't care about the actual order, just that there is one.
        // So it should all work out.
        // Why bother with this? Because it makes the implementation that
        // much simpler :)
        assert_eq!(sid[0], 1, "Expected SID revision to be 1");

        Some(Self { sid })
    }

    /// Retrieves the account name of this SID.
    #[cfg(feature = "user")]
    pub(crate) fn account_name(&self) -> Option<String> {
        let (name, _domain) = self.account_name_and_domain()?;
        Some(name)
    }

    /// Retrieves both the account name and domain of this SID.
    #[cfg(feature = "user")]
    pub(crate) fn account_name_and_domain(&self) -> Option<(String, Option<String>)> {
        unsafe {
            let mut name_len = 0;
            let mut domain_len = 0;
            let mut name_use = SidTypeUnknown;

            let sid = PSID((self.sid.as_ptr() as *mut u8).cast());
            if let Err(err) = LookupAccountSidW(
                PCWSTR::null(),
                sid,
                None,
                &mut name_len,
                None,
                &mut domain_len,
                &mut name_use,
            ) {
                if err.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() {
                    sysinfo_debug!("LookupAccountSidW failed: {:?}", err);
                    return None;
                }
            }

            let mut name = vec![0; name_len as usize];
            let mut domain = vec![0; domain_len as usize];

            if LookupAccountSidW(
                PCWSTR::null(),
                sid,
                Some(PWSTR::from_raw(name.as_mut_ptr())),
                &mut name_len,
                Some(PWSTR::from_raw(domain.as_mut_ptr())),
                &mut domain_len,
                &mut name_use,
            )
            .is_err()
            {
                sysinfo_debug!(
                    "LookupAccountSidW failed: {:?}",
                    std::io::Error::last_os_error()
                );
                return None;
            }

            let username = to_utf8_str(PWSTR::from_raw(name.as_mut_ptr()));
            let domain_name = if domain_len > 1 {
                let domain_str = to_utf8_str(PWSTR::from_raw(domain.as_mut_ptr()));
                if domain_str.is_empty() {
                    None
                } else {
                    Some(domain_str)
                }
            } else {
                None
            };

            Some((username, domain_name))
        }
    }
}

impl Display for Sid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe fn convert_sid_to_string_sid(sid: PSID) -> Option<String> {
            let mut string_sid = PWSTR::null();
            if let Err(_err) = ConvertSidToStringSidW(sid, &mut string_sid) {
                sysinfo_debug!("ConvertSidToStringSidW failed: {:?}", _err);
                return None;
            }
            let result = to_utf8_str(string_sid);
            let _err = LocalFree(Some(HLOCAL(string_sid.0 as _)));
            Some(result)
        }

        let string_sid =
            unsafe { convert_sid_to_string_sid(PSID((self.sid.as_ptr() as *mut u8).cast())) };
        let string_sid = string_sid.ok_or(std::fmt::Error)?;

        write!(f, "{string_sid}")
    }
}

impl FromStr for Sid {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        unsafe {
            let mut string_sid: Vec<u16> = s.encode_utf16().collect();
            string_sid.push(0);

            let mut psid = PSID::default();
            if let Err(err) =
                ConvertStringSidToSidW(PCWSTR::from_raw(string_sid.as_ptr()), &mut psid)
            {
                return Err(format!("ConvertStringSidToSidW failed: {err:?}"));
            }
            let sid = Self::from_psid(psid);
            let _err = LocalFree(Some(HLOCAL(psid.0 as _)));

            // Unwrapping because ConvertStringSidToSidW should've performed
            // all the necessary validations. If it returned an invalid SID,
            // we better fail fast.
            Ok(sid.unwrap())
        }
    }
}
