import { Amplify } from 'aws-amplify';

// AWS Amplify configuration
export const amplifyConfig = {
	Auth: {
		Cognito: {
			region: 'eu-central-1',
			userPoolId: import.meta.env.PUBLIC_USER_POOL_ID,
			userPoolClientId: import.meta.env.PUBLIC_USER_POOL_CLIENT_ID,
		}
	}
};

// Configure Amplify once
let isConfigured = false;

export function configureAmplify() {
	if (!isConfigured) {
		Amplify.configure(amplifyConfig);
		isConfigured = true;
		console.log('Amplify configured successfully');
	}
}
