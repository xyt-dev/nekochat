use super::*;
use reqwest::dns::{Name,Resolve,Resolving};

struct BlockedProviderDns(Arc<Mutex<Vec<String>>>);
impl Resolve for BlockedProviderDns {
    fn resolve(&self,name: Name) -> Resolving {
        self.0.lock().unwrap().push(name.as_str().to_owned());
        Box::pin(async { Err(std::io::Error::other("test transport intentionally blocked").into()) })
    }
}

#[tokio::test]
async fn remote_deepseek_fetch_reaches_only_provider_transport_without_source_dns_preflight() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let hosts = Arc::new(Mutex::new(Vec::new()));
    Arc::get_mut(&mut engine.inner).unwrap().http = reqwest::Client::builder()
        .no_proxy().retry(reqwest::retry::never()).redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(BlockedProviderDns(hosts.clone()))).build().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    engine.inner.vault.lock().unwrap().replace(Service {
        id:id.clone(),name:"Remote fetch".to_owned(),kind:ServiceKind::Deepseek,
        enabled:true,auth_mode:AuthMode::ApiKey,model:None,api_key:Some("not-a-real-key".to_owned()),
    }).unwrap();
    let request = FetchRequest {
        request_id:uuid::Uuid::new_v4().to_string(),service_id:id,
        // Provider-side DNS, not this host's NXDOMAIN/Fake-IP policy, owns remote retrieval.
        url:format!("https://absent-{}.example.com/article",uuid::Uuid::new_v4()),
    };
    let error = fetch(State(engine.inner.clone()),Ok(Json(request))).await.unwrap_err();
    assert_eq!(error.code(),"provider_transport_failed");
    assert_eq!(*hosts.lock().unwrap(),vec!["api.deepseek.com"]);
}
