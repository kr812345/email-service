// pm2 process definition for email_service.
// The binary loads its own configuration from ./.env (dotenvy),
// so only cwd matters here.
module.exports = {
  apps: [
    {
      name: "email-service",
      script: `${__dirname}/target/release/email_service`,
      cwd: __dirname,
      interpreter: "none",
      autorestart: true,
      max_restarts: 10,
      restart_delay: 1000,
      time: true, // timestamps in pm2 logs
    },
  ],
};
