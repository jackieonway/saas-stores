use serde_json::json;

use super::*;

fn auth(resource: Resource, action: Action, user: Uuid) -> Authorized {
    let now = policy::now_millis();
    let tenant = Uuid::from_u128(2);
    let org = Uuid::from_u128(3);
    let intro = json!({"active":true,"aud":"stores","sub":user,"tenant_id":tenant,
        "organization_id":org,"token_type":"Bearer","exp":now/1000+120});
    let context = json!({"contractVersion":"1.0","appCode":"stores","contextVersion":"a".repeat(64),
        "expiresAt":chrono::DateTime::from_timestamp_millis(now+20_000).unwrap().to_rfc3339(),
        "user":{"id":user},"tenant":{"id":tenant},"organization":{"id":org},
        "application":{"id":Uuid::from_u128(4)},
        "permissions":[format!("{}:{}",resource.as_str(),action.as_str())],
        "dataGrants":[{"resource":resource.as_str(),"action":action.as_str(),"tenantId":tenant,
        "subjectUserId":user,"scope":"ORG","organizationIds":[org]}]});
    policy::validate_context(
        &policy::validate_intro(&intro, now).unwrap(),
        &context,
        resource,
        action,
        now,
    )
    .unwrap()
}

#[test]
fn member_witness_is_exactly_bound_and_deadline_does_not_renew() {
    let user = Uuid::from_u128(1);
    let auth = auth(Resource::Store, Action::Create, user);
    let organization = auth.organization_id();
    let target = Uuid::from_u128(5);
    let start = Instant::now();
    let witness = VerifiedMember {
        actor: user,
        tenant: auth.tenant_id(),
        current_organization: organization,
        resource: Resource::Store,
        action: Action::Create,
        organization,
        user: target,
        display_name: "有效成员".into(),
        deadline: start + Duration::from_secs(3),
    };
    witness.assert_fresh(&auth, organization, target).unwrap();
    for changed in [
        self::auth(Resource::Staff, Action::Assign, user),
        self::auth(Resource::Store, Action::Update, user),
        self::auth(Resource::Store, Action::Create, target),
    ] {
        assert!(witness
            .assert_fresh(&changed, organization, target)
            .is_err());
    }
    assert!(witness.assert_fresh(&auth, organization, user).is_err());
    assert!(witness.assert_fresh(&auth, target, target).is_err());
    witness
        .assert_at(start + Duration::from_millis(2999))
        .unwrap();
    assert_eq!(
        witness
            .assert_at(start + Duration::from_secs(3))
            .unwrap_err()
            .code(),
        "MEMBER_VALIDATION_EXPIRED"
    );
    assert_eq!(witness.deadline, start + Duration::from_secs(3));
    assert_eq!(witness.user_id(), target);
    assert_eq!(witness.organization_id(), organization);
    assert_eq!(witness.display_name(), "有效成员");
}

#[test]
fn directory_purpose_and_member_wire_are_strict() {
    for (resource, action, allowed) in [
        (Resource::Store, Action::Create, true),
        (Resource::Store, Action::Update, true),
        (Resource::Staff, Action::Assign, true),
        (Resource::Staff, Action::Unassign, false),
        (Resource::Store, Action::Read, false),
        (Resource::DailyReport, Action::Create, false),
    ] {
        let auth = auth(resource, action, Uuid::from_u128(1));
        assert_eq!(
            directory_authorized(&auth, auth.organization_id()).is_ok(),
            allowed
        );
    }
    let org = Uuid::from_u128(3);
    let original = json!({"id":Uuid::from_u128(5),"organizationId":org,"displayName":"成员"});
    validate_member(&original, org).unwrap();
    for (field, replacement) in [
        ("id", json!("invalid")),
        ("organizationId", json!(Uuid::from_u128(9))),
        ("displayName", json!("  ")),
        ("displayName", json!("x".repeat(101))),
        ("displayName", json!("a\0b")),
    ] {
        let mut value = original.clone();
        value[field] = replacement;
        assert_eq!(
            validate_member(&value, org).unwrap_err().code(),
            "PLATFORM_UNAVAILABLE"
        );
    }
    let page = json!({"records":[original.clone()],"total":1,"page":1,"size":20});
    assert_eq!(validate_page(&page, org, 1, 20).unwrap(), page);
    let mut extended = page.clone();
    extended["unexpectedPrivateField"] = json!("upstream-only");
    extended["records"][0]["unexpectedPrivateField"] = json!("upstream-only");
    assert_eq!(validate_page(&extended, org, 1, 20).unwrap(), page);
    for (field, replacement) in [
        ("page", json!(2)),
        ("size", json!(50)),
        ("total", json!(-1)),
        ("total", json!(9_007_199_254_740_992_u64)),
        ("records", json!([original.clone(), original])),
    ] {
        let mut value = page.clone();
        value[field] = replacement;
        assert!(validate_page(&value, org, 1, 20).is_err());
    }
}
