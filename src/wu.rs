//! Windows Update Agent (COM) : recherche et installation des pilotes.
use crate::model::{Update, ole_date};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::UpdateAgent::*;
use windows::core::{BSTR, Interface, Result};

pub struct Wu {
    session: IUpdateSession,
    found: Vec<IUpdate>,
}

impl Wu {
    pub fn new() -> Result<Self> {
        let session = unsafe { CoCreateInstance(&UpdateSession, None, CLSCTX_INPROC_SERVER)? };
        Ok(Self { session, found: Vec::new() })
    }

    /// Pilotes proposés par Windows Update pour CE PC (le matching matériel est fait par Windows).
    pub fn search(&mut self) -> Result<Vec<Update>> {
        unsafe {
            let searcher = self.session.CreateUpdateSearcher()?;
            let result = searcher.Search(&BSTR::from("IsInstalled=0 and Type='Driver'"))?;
            let list = result.Updates()?;
            self.found.clear();
            let mut out = Vec::new();
            for i in 0..list.Count()? {
                let u = list.get_Item(i)?;
                let d: IWindowsDriverUpdate = u.cast()?;
                out.push(Update {
                    title: u.Title()?.to_string(),
                    provider: d.DriverProvider()?.to_string(),
                    hwid: d.DriverHardwareID()?.to_string(),
                    date: ole_date(d.DriverVerDate()?),
                });
                self.found.push(u);
            }
            Ok(out)
        }
    }

    /// Télécharge puis installe les mises à jour aux index donnés. Renvoie un résumé lisible.
    pub fn install(&self, picks: &[usize]) -> Result<String> {
        unsafe {
            let coll: IUpdateCollection = CoCreateInstance(&UpdateCollection, None, CLSCTX_INPROC_SERVER)?;
            for &i in picks {
                let Some(u) = self.found.get(i) else { continue };
                if !u.EulaAccepted()?.as_bool() {
                    u.AcceptEula()?;
                }
                coll.Add(u)?;
            }
            if coll.Count()? == 0 {
                return Ok("Rien à installer.".into());
            }

            let dl = self.session.CreateUpdateDownloader()?;
            dl.SetUpdates(&coll)?;
            dl.Download()?;

            let inst = self.session.CreateUpdateInstaller()?;
            inst.SetUpdates(&coll)?;
            let res = inst.Install()?;

            let n = coll.Count()?;
            let ok = (0..n)
                .filter(|&i| res.GetUpdateResult(i).and_then(|r| r.ResultCode()).is_ok_and(|c| c == orcSucceeded))
                .count();
            let reboot = if res.RebootRequired()?.as_bool() { " Redémarrage nécessaire." } else { "" };
            Ok(format!("{ok}/{n} pilote(s) installé(s).{reboot}"))
        }
    }
}
