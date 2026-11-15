//! Windows Update Agent (COM) : recherche et installation des pilotes.
use crate::model::{Source, Update, ole_date};
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
            let result = searcher.Search(&BSTR::from("IsInstalled=0 and Type='Driver' and IsHidden=0"))?;
            let list = result.Updates()?;
            self.found.clear();
            let mut out = Vec::new();
            for i in 0..list.Count()? {
                let u = list.get_Item(i)?;
                let d: IWindowsDriverUpdate = u.cast()?;
                out.push(Update {
                    source: Source::Wu(self.found.len()),
                    version: String::new(),
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

    pub fn title(&self, i: usize) -> String {
        self.found.get(i).and_then(|u| unsafe { u.Title() }.ok()).map_or_else(|| format!("pilote n°{i}"), |t| t.to_string())
    }

    /// Télécharge puis installe la mise à jour n° `i` de la dernière recherche.
    /// Renvoie (réussi, redémarrage nécessaire).
    pub fn install_one(&self, i: usize) -> Result<(bool, bool)> {
        let Some(u) = self.found.get(i) else { return Ok((false, false)) };
        unsafe {
            if !u.EulaAccepted()?.as_bool() {
                u.AcceptEula()?;
            }
            let coll: IUpdateCollection = CoCreateInstance(&UpdateCollection, None, CLSCTX_INPROC_SERVER)?;
            coll.Add(u)?;

            let dl = self.session.CreateUpdateDownloader()?;
            dl.SetUpdates(&coll)?;
            dl.Download()?;

            let inst = self.session.CreateUpdateInstaller()?;
            inst.SetUpdates(&coll)?;
            let res = inst.Install()?;
            Ok((res.ResultCode()? == orcSucceeded, res.RebootRequired()?.as_bool()))
        }
    }
}
