use jni::{
    Env, jni_sig, jni_str,
    objects::{Global, JObject, JValue},
};

use super::envelope::{Delivery, PreparedEnvelope};

pub struct FrameworkChannel {
    manager: Global<JObject<'static>>,
    identity: String,
    profile: i32,
}

impl FrameworkChannel {
    pub fn attach(env: &mut Env, profile: i32, identity: &str) -> jni::errors::Result<Self> {
        let registry = env.find_class(jni_str!("android/os/ServiceManager"))?;
        let service_name = env.new_string("activity")?;
        let binder = env
            .call_static_method(
                registry,
                jni_str!("getService"),
                jni_sig!((name: java.lang.String) -> android.os.IBinder),
                &[JValue::Object(&service_name.into())],
            )?
            .l()?;
        let adapter = env.find_class(jni_str!("android/app/IActivityManager$Stub"))?;
        let manager = env
            .call_static_method(
                adapter,
                jni_str!("asInterface"),
                jni_sig!((binder: android.os.IBinder) -> android.app.IActivityManager),
                &[JValue::Object(&binder)],
            )?
            .l()?;
        Ok(Self {
            manager: env.new_global_ref(manager)?,
            identity: identity.to_string(),
            profile,
        })
    }

    pub fn transmit(
        &self,
        env: &mut Env,
        envelope: PreparedEnvelope<'_>,
    ) -> jni::errors::Result<()> {
        match envelope.delivery {
            Delivery::Background => self.background(env, &envelope.value),
            Delivery::Foreground(mime) => self.foreground(env, &envelope.value, &mime),
        }
    }

    fn background(&self, env: &mut Env, value: &JObject<'_>) -> jni::errors::Result<()> {
        let identity = env.new_string(&self.identity)?;
        if env
            .call_method(
                self.manager.as_obj(),
                jni_str!("startService"),
                jni_sig!((
                    caller: android.app.IApplicationThread,
                    service: android.content.Intent,
                    resolved_type: java.lang.String,
                    require_foreground: boolean,
                    calling_package: java.lang.String,
                    calling_feature_id: java.lang.String,
                    user_id: int,
                ) -> android.content.ComponentName),
                &[
                    JValue::Object(&JObject::null()),
                    JValue::Object(value),
                    JValue::Object(&JObject::null()),
                    JValue::Bool(false),
                    JValue::Object(&identity.into()),
                    JValue::Object(&JObject::null()),
                    JValue::Int(self.profile),
                ],
            )
            .is_ok()
        {
            return Ok(());
        }
        let identity = env.new_string(&self.identity)?;
        env.call_method(
            self.manager.as_obj(),
            jni_str!("startService"),
            jni_sig!((
                caller: android.app.IApplicationThread,
                service: android.content.Intent,
                resolved_type: java.lang.String,
                require_foreground: boolean,
                calling_package: java.lang.String,
                user_id: int,
            ) -> android.content.ComponentName),
            &[
                JValue::Object(&JObject::null()),
                JValue::Object(value),
                JValue::Object(&JObject::null()),
                JValue::Bool(false),
                JValue::Object(&identity.into()),
                JValue::Int(self.profile),
            ],
        )?;
        Ok(())
    }

    fn foreground(
        &self,
        env: &mut Env,
        value: &JObject<'_>,
        mime: &str,
    ) -> jni::errors::Result<()> {
        let identity = env.new_string(&self.identity)?;
        let mime_value = env.new_string(mime)?;
        if env
            .call_method(
                self.manager.as_obj(),
                jni_str!("startActivity"),
                jni_sig!((
                    caller: android.app.IApplicationThread,
                    calling_package: java.lang.String,
                    calling_feature_id: java.lang.String,
                    intent: android.content.Intent,
                    resolved_type: java.lang.String,
                    result_to: android.os.IBinder,
                    result_who: java.lang.String,
                    request_code: int,
                    flags: int,
                    profiler_info: android.app.ProfilerInfo,
                    options: android.os.Bundle,
                    user_id: int,
                ) -> int),
                &[
                    JValue::Object(&JObject::null()),
                    JValue::Object(&identity.into()),
                    JValue::Object(&JObject::null()),
                    JValue::Object(value),
                    JValue::Object(&mime_value.into()),
                    JValue::Object(&JObject::null()),
                    JValue::Object(&JObject::null()),
                    JValue::Int(0),
                    JValue::Int(0),
                    JValue::Object(&JObject::null()),
                    JValue::Object(&JObject::null()),
                    JValue::Int(self.profile),
                ],
            )
            .is_ok()
        {
            return Ok(());
        }
        let identity = env.new_string(&self.identity)?;
        let mime_value = env.new_string(mime)?;
        env.call_method(
            self.manager.as_obj(),
            jni_str!("startActivityAsUser"),
            jni_sig!((
                caller: android.app.IApplicationThread,
                calling_package: java.lang.String,
                intent: android.content.Intent,
                resolved_type: java.lang.String,
                result_to: android.os.IBinder,
                result_who: java.lang.String,
                request_code: int,
                flags: int,
                profiler_info: android.app.ProfilerInfo,
                options: android.os.Bundle,
                user_id: int,
            ) -> int),
            &[
                JValue::Object(&JObject::null()),
                JValue::Object(&identity.into()),
                JValue::Object(value),
                JValue::Object(&mime_value.into()),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
                JValue::Int(0),
                JValue::Int(0),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
                JValue::Int(self.profile),
            ],
        )?;
        Ok(())
    }
}
